// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the tests of the crate share: a graph given by its pairs (the model the
//! answers are checked against), stored in every source the edge-map framework reads,
//! and a run of one query on all of them, which must agree.

#![allow(dead_code, reason = "each test uses part of it")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test code: small graphs, files for the mapped and streamed sources"
)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use everygraph_core::{AccessHint, ByteSource, Memory, ReadError};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::edge_map::{Compressed, CompressedStream, Csr, RawCopy, Source, Stream};
use everygraph_query::{Columns, Graph};

/// What a pair carries: its events, the sum of their weights, the first and last time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Agg {
    pub edges: u64,
    pub weight: f64,
    pub first: i64,
    pub last: i64,
}

impl Agg {
    fn add(&mut self, w: f64, t: i64) {
        self.edges += 1;
        self.weight += w;
        self.first = self.first.min(t);
        self.last = self.last.max(t);
    }

    /// The two directions of a pair, as `Both` reports them.
    pub fn and(self, o: Self) -> Self {
        Self {
            edges: self.edges + o.edges,
            weight: self.weight + o.weight,
            first: self.first.min(o.first),
            last: self.last.max(o.last),
        }
    }
}

/// A graph given by its pairs, as its lists store them: on an undirected graph both
/// `(u, v)` and `(v, u)`, a loop once.
#[derive(Clone, Debug)]
pub struct Model {
    pub n: u32,
    pub directed: bool,
    pub out: BTreeMap<(u32, u32), Agg>,
    pub inn: BTreeMap<(u32, u32), Agg>,
}

impl Model {
    /// The graph of `rows` (`source`, `target`, weight, time): every row an event, every
    /// loop kept.
    pub fn new(n: u32, directed: bool, rows: &[(u32, u32, f64, i64)]) -> Self {
        let empty = |t| Agg {
            edges: 0,
            weight: 0.0,
            first: t,
            last: t,
        };
        let mut out = BTreeMap::new();
        for &(s, d, w, t) in rows {
            out.entry((s, d)).or_insert_with(|| empty(t)).add(w, t);
            if !directed && s != d {
                out.entry((d, s)).or_insert_with(|| empty(t)).add(w, t);
            }
        }
        let inn = out.iter().map(|(&(s, d), &a)| ((d, s), a)).collect();
        Self {
            n,
            directed,
            out,
            inn,
        }
    }

    /// The out-list (`inn` false) or the in-list of `v`.
    pub fn list(&self, v: u32, inn: bool) -> Vec<(u32, Agg)> {
        let map = if inn && self.directed {
            &self.inn
        } else {
            &self.out
        };
        map.range((v, 0)..=(v, u32::MAX))
            .map(|(&(_, u), &a)| (u, a))
            .collect()
    }

    /// The neighbours of `v` in the undirected projection, a loop once: out and in
    /// merged, each pair the sum of both directions.
    pub fn both(&self, v: u32) -> Vec<(u32, Agg)> {
        let mut all: BTreeMap<u32, Agg> = self.list(v, false).into_iter().collect();
        if self.directed {
            for (u, a) in self.list(v, true) {
                if u == v {
                    continue;
                }
                all.entry(u).and_modify(|x| *x = x.and(a)).or_insert(a);
            }
        }
        all.into_iter().collect()
    }

    fn lists(&self, inn: bool) -> Lists {
        let mut l = Lists::default();
        l.offsets.push(0);
        for v in 0..self.n {
            for (u, a) in self.list(v, inn) {
                l.targets.push(u);
                l.edges.push(a.edges);
                l.weight.push(a.weight);
                l.first.push(a.first);
                l.last.push(a.last);
            }
            l.offsets.push(l.targets.len() as u64);
        }
        l
    }
}

/// One adjacency as a raw CSR with its per-entry columns.
#[derive(Clone, Debug, Default)]
pub struct Lists {
    pub offsets: Vec<u64>,
    pub targets: Vec<u32>,
    pub edges: Vec<u64>,
    pub weight: Vec<f64>,
    pub first: Vec<i64>,
    pub last: Vec<i64>,
}

/// A byte source that counts what is read from it.
pub struct Counted<S> {
    pub inner: S,
    pub reads: AtomicU64,
}

impl<S: ByteSource> ByteSource for Counted<S> {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), ReadError> {
        self.reads.fetch_add(buf.len() as u64, Ordering::Relaxed);
        self.inner.read_at(offset, buf)
    }

    fn advise(&self, range: std::ops::Range<u64>, hint: AccessHint) {
        self.inner.advise(range, hint);
    }
}

/// One adjacency in every storage the sources read.
pub struct Stored {
    pub lists: Lists,
    pub index: AdjacencyIndex,
    pub data: Vec<u8>,
    pub raw: RawCopy,
    map: MmapSource,
    pub file: Counted<FileSource>,
    stream: FileSource,
    files: [PathBuf; 2],
}

impl Drop for Stored {
    fn drop(&mut self) {
        for f in &self.files {
            let _ = std::fs::remove_file(f);
        }
    }
}

impl Stored {
    pub fn new(lists: Lists) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("query");
        std::fs::create_dir_all(&dir).unwrap();
        let k = NEXT.fetch_add(1, Ordering::Relaxed);
        let id = std::process::id();
        let files = [
            dir.join(format!("{id}-{k}.data")),
            dir.join(format!("{id}-{k}.targets")),
        ];
        let encoded = encode(&lists.offsets, &lists.targets).unwrap();
        std::fs::write(&files[0], &encoded.data).unwrap();
        let bytes: Vec<u8> = lists.targets.iter().flat_map(|t| t.to_le_bytes()).collect();
        std::fs::write(&files[1], bytes).unwrap();
        let index = AdjacencyIndex::parse(&encoded.index).unwrap();
        let raw = RawCopy::decode(
            &Adjacency::new(&index, &encoded.data).unwrap(),
            &Memory::unlimited(),
        )
        .unwrap();
        Self {
            lists,
            index,
            data: encoded.data,
            raw,
            map: MmapSource::open(&files[0]).unwrap(),
            file: Counted {
                inner: FileSource::open(&files[0]).unwrap(),
                reads: AtomicU64::new(0),
            },
            stream: FileSource::open(&files[1]).unwrap(),
            files,
        }
    }

    pub fn columns(&self) -> Columns<'_> {
        Columns {
            edges: Some(&self.lists.edges),
            weight: Some(&self.lists.weight),
            first: Some(&self.lists.first),
            last: Some(&self.lists.last),
            ..Columns::default()
        }
    }

    pub fn csr(&self) -> Csr<'_> {
        Csr::new(&self.lists.offsets, &self.lists.targets).unwrap()
    }

    pub fn heap(&self) -> Compressed<'_> {
        Compressed::new(Adjacency::new(&self.index, &self.data).unwrap())
    }

    pub fn mapped(&self) -> Compressed<'_> {
        Compressed::mapped(&self.index, &self.map).unwrap()
    }

    pub fn stream(&self) -> Stream<'_> {
        Stream::new(&self.lists.offsets, &self.stream).unwrap()
    }

    /// The compressed lists read through the file in the smallest window.
    pub fn streamed(&self) -> CompressedStream<'_> {
        CompressedStream::new(&self.index, &self.file, 0).unwrap()
    }
}

/// A model stored in every source: its out-lists and, if directed, its in-lists.
pub struct Fixture {
    pub model: Model,
    pub out: Stored,
    pub inn: Option<Stored>,
}

impl Fixture {
    pub fn new(model: Model) -> Self {
        let inn = model.directed.then(|| Stored::new(model.lists(true)));
        Self {
            out: Stored::new(model.lists(false)),
            inn,
            model,
        }
    }

    /// The query graph over `out` and `inn`, with every column.
    pub fn graph<'a, S: Source>(&'a self, out: &'a S, inn: Option<&'a S>) -> Graph<'a, S> {
        let g = if self.model.directed {
            Graph::directed(out, inn).unwrap()
        } else {
            Graph::undirected(out)
        };
        let in_columns = self.inn.as_ref().map(Stored::columns).unwrap_or_default();
        g.with_columns(self.out.columns(), in_columns).unwrap()
    }
}

/// A query, run on any source.
pub trait Query {
    type Out: PartialEq + Debug;
    fn run<S: Source>(&self, g: &Graph<'_, S>) -> Self::Out;
}

/// `q` on every source of `f`; every answer must be the first one, which is returned.
pub fn on_every_source<Q: Query>(f: &Fixture, q: &Q) -> Q::Out {
    let inn = f.inn.as_ref();
    let answers = [
        (
            "csr",
            q.run(&f.graph(&f.out.csr(), inn.map(Stored::csr).as_ref())),
        ),
        ("rawcopy", q.run(&f.graph(&f.out.raw, inn.map(|s| &s.raw)))),
        (
            "heap",
            q.run(&f.graph(&f.out.heap(), inn.map(Stored::heap).as_ref())),
        ),
        (
            "mapped",
            q.run(&f.graph(&f.out.mapped(), inn.map(Stored::mapped).as_ref())),
        ),
        (
            "stream",
            q.run(&f.graph(&f.out.stream(), inn.map(Stored::stream).as_ref())),
        ),
        (
            "streamed",
            q.run(&f.graph(&f.out.streamed(), inn.map(Stored::streamed).as_ref())),
        ),
    ];
    let mut answers = answers.into_iter();
    let (_, first) = answers.next().unwrap();
    for (name, a) in answers {
        assert_eq!(a, first, "{name} differs from csr");
    }
    first
}
