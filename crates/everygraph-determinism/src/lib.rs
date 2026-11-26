// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The determinism suite (issue #54, DESIGN D14): a deterministic operation gives the
//! same bits at any thread count, chunking, source and repetition.
//!
//! - [`grid`]: the configurations a result must not depend on: 1, 2, 3, 8 and 32
//!   threads (32 oversubscribes a CI runner's 4 cores), tasks of 64 and 65,536 entries,
//!   and the four sources of the edge-map framework ([`Kind`]).
//! - [`same`] runs one operation at every configuration and fails with every
//!   configuration whose result differs from the first.
//! - [`kernels`] and [`communities`] hash the output columns of every whole-graph
//!   kernel and of Leiden (BLAKE3 over the little-endian bytes, floats by their bits).
//!
//! On a pull request the grid is a sample in which every thread count, chunking and
//! source appears; with `EVERYGRAPH_DETERMINISM=full` (the nightly job of
//! `.github/workflows/determinism.yml`) it is every combination, run twice.

#![forbid(unsafe_code)]
// A test tool: small buffers outside any budget, and generated graphs whose ids fit u32.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::cast_possible_truncation,
    clippy::missing_panics_doc,
    missing_debug_implementations
)]

use std::fmt::Debug;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use everygraph_community::{Communities, Options, leiden};
use everygraph_core::{Memory, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_io::{FileSource, MmapSource};
use everygraph_kernels::edge_map::{Compressed, Csr, Plan, Settings, Source, Stream};
use everygraph_kernels::pagerank::{self, Eigenvector, PageRank};
use everygraph_kernels::run::{self, Column, Operation, Precision};
use everygraph_kernels::sssp::{Sssp, sssp};
use everygraph_oracle::corpus::{Tier, entries};
use everygraph_oracle::projection::Projection;

/// Whether the whole grid runs: `EVERYGRAPH_DETERMINISM=full`.
#[must_use]
pub fn full() -> bool {
    std::env::var("EVERYGRAPH_DETERMINISM").is_ok_and(|v| v == "full")
}

/// Thread counts.
pub const THREADS: [usize; 5] = [1, 2, 3, 8, 32];
/// Entries per task; each comes with its own decode batch.
pub const CHUNKS: [(u64, usize); 2] = [(64, 7), (1 << 16, 8192)];

/// Where the lists come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Raw CSR arrays in memory.
    Raw,
    /// The adjacency codec over a heap buffer.
    Heap,
    /// The adjacency codec over a memory map.
    Mapped,
    /// Raw targets read from a file.
    Streamed,
}

/// Every kind.
pub const KINDS: [Kind; 4] = [Kind::Raw, Kind::Heap, Kind::Mapped, Kind::Streamed];

/// One way to run an operation.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Threads of the pool.
    pub threads: usize,
    /// Entries per task, and per decode batch.
    pub chunk: (u64, usize),
    /// The source.
    pub kind: Kind,
}

impl Config {
    /// The plan's settings.
    #[must_use]
    pub fn settings(&self) -> Settings {
        Settings {
            chunk_entries: self.chunk.0,
            batch_entries: self.chunk.1,
            prefetch: false,
        }
    }
}

/// The configurations: every combination, twice, when [`full`]; else each thread
/// count once, with the chunkings and kinds turned by `turn` so that a test over many
/// graphs meets every pair, and the first configuration again at the end.
#[must_use]
pub fn grid(turn: usize) -> Vec<Config> {
    let mut out = Vec::new();
    if full() {
        for _ in 0..2 {
            for threads in THREADS {
                for chunk in CHUNKS {
                    for kind in KINDS {
                        out.push(Config {
                            threads,
                            chunk,
                            kind,
                        });
                    }
                }
            }
        }
        return out;
    }
    for (i, threads) in THREADS.into_iter().enumerate() {
        out.push(Config {
            threads,
            chunk: CHUNKS[(i + turn) % CHUNKS.len()],
            kind: KINDS[(i + turn / 2) % KINDS.len()],
        });
    }
    out.push(out[0]);
    out
}

/// `f` on a pool of `threads` threads.
pub fn on<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    par::with_threads(NonZeroUsize::new(threads).expect("threads"), f).expect("a pool")
}

/// Runs `f` at every configuration of `grid`, each on its own pool, and returns the
/// first result.
///
/// # Panics
///
/// With the name and configuration of every result that differs from the first.
pub fn same<T: PartialEq + Debug + Send>(
    what: &str,
    grid: &[Config],
    f: impl Fn(Config) -> T + Sync,
) -> T {
    let mut results = grid.iter().map(|&c| (c, on(c.threads, || f(c))));
    let (first_config, first) = results.next().expect("a configuration");
    let differ: Vec<String> = results
        .filter(|(_, r)| *r != first)
        .map(|(c, r)| format!("{c:?}: {r:?}"))
        .collect();
    assert!(
        differ.is_empty(),
        "{what}: differs from {first_config:?} ({first:?}) at\n{}",
        differ.join("\n")
    );
    first
}

/// A 64-bit generator for test data (`SplitMix64`).
#[derive(Clone, Debug)]
pub struct SplitMix(pub u64);

impl SplitMix {
    /// The next value.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value below `n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// A weight log-uniform over 2^-20 to 2^21: a random exponent and mantissa. Such
    /// weights make a float sum depend on its order, where unit weights do not
    /// (`docs/research/sota-numerics.md`, R15a).
    pub fn wide_weight(&mut self) -> f64 {
        let exponent = 1023 - 20 + self.below(41);
        f64::from_bits(exponent << 52 | self.next_u64() >> 12)
    }
}

/// Lists of an undirected graph: each pair in both lists, a kept loop once, and one
/// weight per entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Lists {
    /// `n + 1` offsets.
    pub offsets: Vec<u64>,
    /// The neighbours, increasing in each list.
    pub targets: Vec<u32>,
    /// The weight of each entry: the pair's.
    pub weights: Vec<f64>,
}

impl Lists {
    /// From pairs and their weights; of a repeated pair the first is kept.
    #[must_use]
    pub fn undirected(n: u32, pairs: impl IntoIterator<Item = (u32, u32, f64)>) -> Self {
        let mut entries: Vec<(u32, u32, f64)> = pairs
            .into_iter()
            .flat_map(|(u, v, w)| [(u, v, w), (v, u, w)])
            .collect();
        entries.sort_by_key(|e| (e.0, e.1));
        entries.dedup_by_key(|e| (e.0, e.1));
        let mut offsets = vec![0_u64; n as usize + 1];
        for e in &entries {
            offsets[e.0 as usize + 1] += 1;
        }
        for v in 0..n as usize {
            offsets[v + 1] += offsets[v];
        }
        Self {
            offsets,
            targets: entries.iter().map(|e| e.1).collect(),
            weights: entries.iter().map(|e| e.2).collect(),
        }
    }

    /// The node count.
    #[must_use]
    pub fn nodes(&self) -> u32 {
        (self.offsets.len() - 1) as u32
    }

    /// Each pair once (`u <= v`), with its weight.
    #[must_use]
    pub fn pairs(&self) -> Vec<(u32, u32, f64)> {
        let mut out = Vec::new();
        for u in 0..self.nodes() {
            let r = self.offsets[u as usize] as usize..self.offsets[u as usize + 1] as usize;
            for (&v, &w) in self.targets[r.clone()].iter().zip(&self.weights[r]) {
                if v >= u {
                    out.push((u, v, w));
                }
            }
        }
        out
    }

    /// The graph with node `v` renamed `to[v]`, over `n` nodes; the pairs of a node
    /// whose new name is `None` are dropped.
    #[must_use]
    pub fn renamed(&self, n: u32, to: &[Option<u32>]) -> Self {
        let pairs = self
            .pairs()
            .into_iter()
            .filter_map(|(u, v, w)| Some((to[u as usize]?, to[v as usize]?, w)));
        Self::undirected(n, pairs)
    }
}

/// `blocks` blocks of `size` nodes: each node draws `d_in` neighbours in its block and
/// `d_out` anywhere, loops dropped. Weighted with [`SplitMix::wide_weight`].
#[must_use]
pub fn planted(blocks: u32, size: u32, d_in: u32, d_out: u32, seed: u64) -> Lists {
    let mut rng = SplitMix(seed);
    let n = blocks * size;
    let mut pairs = Vec::new();
    for u in 0..n {
        let block = u / size * size;
        for i in 0..d_in + d_out {
            let v = if i < d_in {
                block + rng.below(u64::from(size)) as u32
            } else {
                rng.below(u64::from(n)) as u32
            };
            let w = rng.wide_weight();
            if v != u {
                pairs.push((u, v, w));
            }
        }
    }
    Lists::undirected(n, pairs)
}

/// The undirected projections of the oracle's CI corpus, kept loops in the lists, every
/// `step`-th graph; unit weights.
#[must_use]
pub fn corpus(step: usize) -> Vec<(String, Lists)> {
    entries(Tier::Ci)
        .into_iter()
        .step_by(step)
        .map(|e| {
            let p = Projection::new(&e.graph);
            let sym = &p.undirected;
            let pairs = (0..sym.n()).flat_map(|u| {
                let loops = (p.kept_loops(u) > 0).then_some((u as u32, u as u32, 1.0));
                sym.list(u)
                    .iter()
                    .filter(move |&&v| v as usize > u)
                    .map(move |&v| (u as u32, v, 1.0))
                    .chain(loops)
            });
            (
                e.graph.name.clone(),
                Lists::undirected(sym.n() as u32, pairs),
            )
        })
        .collect()
}

/// One graph in every storage the sources read: the codec's bytes in the heap and in a
/// mapped file, and the raw targets in a file. The files are removed on drop.
pub struct Stored {
    /// The lists.
    pub lists: Lists,
    index: AdjacencyIndex,
    data: Vec<u8>,
    map: MmapSource,
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

/// A source of any kind; [`with_source!`] runs code generic over [`Source`] on it.
pub enum Any<'a> {
    /// [`Kind::Raw`].
    Raw(Csr<'a>),
    /// [`Kind::Heap`] and [`Kind::Mapped`].
    Compressed(Compressed<'a>),
    /// [`Kind::Streamed`].
    Streamed(Stream<'a>),
}

/// `with_source!(any, |s| body)`: `body` with `s` bound to the source inside `any`.
#[macro_export]
macro_rules! with_source {
    ($any:expr, |$s:ident| $body:expr) => {
        match $any {
            $crate::Any::Raw($s) => $body,
            $crate::Any::Compressed($s) => $body,
            $crate::Any::Streamed($s) => $body,
        }
    };
}

impl Stored {
    /// Writes the files under `dir`.
    #[must_use]
    pub fn new(lists: Lists, dir: &Path) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        std::fs::create_dir_all(dir).unwrap();
        let k = NEXT.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let files = [
            dir.join(format!("determinism-{pid}-{k}.data")),
            dir.join(format!("determinism-{pid}-{k}.targets")),
        ];
        let encoded = encode(&lists.offsets, &lists.targets).unwrap();
        std::fs::write(&files[0], &encoded.data).unwrap();
        let bytes: Vec<u8> = lists.targets.iter().flat_map(|t| t.to_le_bytes()).collect();
        std::fs::write(&files[1], bytes).unwrap();
        Self {
            index: AdjacencyIndex::parse(&encoded.index).unwrap(),
            data: encoded.data,
            map: MmapSource::open(&files[0]).unwrap(),
            stream: FileSource::open(&files[1]).unwrap(),
            files,
            lists,
        }
    }

    /// The source of `kind`.
    #[must_use]
    pub fn source(&self, kind: Kind) -> Any<'_> {
        let l = &self.lists;
        match kind {
            Kind::Raw => Any::Raw(Csr::new(&l.offsets, &l.targets).unwrap()),
            Kind::Heap => Any::Compressed(Compressed::new(
                Adjacency::new(&self.index, &self.data).unwrap(),
            )),
            Kind::Mapped => Any::Compressed(Compressed::mapped(&self.index, &self.map).unwrap()),
            Kind::Streamed => Any::Streamed(Stream::new(&l.offsets, &self.stream).unwrap()),
        }
    }
}

/// A BLAKE3 hash of values in order, as hexadecimal digits.
#[derive(Default)]
pub struct Hash(blake3::Hasher);

impl Hash {
    /// Adds `u32`s.
    pub fn u32s(&mut self, values: impl IntoIterator<Item = u32>) -> &mut Self {
        for x in values {
            self.0.update(&x.to_le_bytes());
        }
        self
    }

    /// Adds `u64`s.
    pub fn u64s(&mut self, values: impl IntoIterator<Item = u64>) -> &mut Self {
        for x in values {
            self.0.update(&x.to_le_bytes());
        }
        self
    }

    /// Adds `f64`s by their bits.
    pub fn f64s(&mut self, values: impl IntoIterator<Item = f64>) -> &mut Self {
        self.u64s(values.into_iter().map(f64::to_bits))
    }

    /// The first 16 hexadecimal digits.
    #[must_use]
    pub fn hex(&self) -> String {
        self.0.finalize().to_hex()[..16].to_owned()
    }
}

/// The hash of a result column.
fn column_hash(c: &Column) -> String {
    let mut h = Hash::default();
    match c {
        Column::U32 { values, .. } => h.u32s(values.iter().copied()),
        Column::F64(values) => h.f64s(values.iter().copied()),
        Column::F32(values) => h.u32s(values.iter().map(|x| x.to_bits())),
    };
    h.hex()
}

/// The hash of each output column of every whole-graph operation of `everygraph run`
/// (`everygraph_kernels::run`), then of weighted `PageRank`, eigenvector centrality and
/// SSSP from node 0 with the per-entry `weights`. `PageRank` and eigenvector run 30
/// iterations, never stopping early, so that a difference has room to grow.
#[must_use]
pub fn kernels<S: Source>(
    source: &S,
    settings: Settings,
    weights: &[f64],
) -> Vec<(&'static str, String)> {
    let memory = Memory::unlimited();
    let fixed = PageRank {
        tol: 0.0,
        max_iter: 30,
        ..PageRank::default()
    };
    let ops = [
        Operation::PageRank {
            params: fixed,
            precision: Precision::Full,
        },
        Operation::PageRank {
            params: fixed,
            precision: Precision::Low,
        },
        Operation::Components,
        Operation::KCore,
        Operation::Degrees,
        Operation::Bfs {
            source: 0,
            max_hops: None,
        },
    ];
    Plan::with(source, &memory, settings, |plan| {
        let mut out = Vec::new();
        if plan.nodes() == 0 {
            return out;
        }
        for op in &ops {
            let output = run::run(op, &plan, false, &memory).unwrap();
            for (name, column) in &output.columns {
                out.push((*name, column_hash(column)));
            }
        }
        let s = pagerank::strength(&plan, Some(weights), false, &memory).unwrap();
        let g = pagerank::Graph {
            lists: &plan,
            weights: Some(weights),
            directed: false,
        };
        let ranks = pagerank::pagerank::<_, f64>(&g, s.as_slice().unwrap(), &fixed, &memory);
        let e = Eigenvector {
            tol: 0.0,
            max_iter: 30,
        };
        let eigen = pagerank::eigenvector(&g, &e, &memory).unwrap();
        let d = sssp(&plan, weights, 0, &Sssp::default(), &memory).unwrap();
        let bits = d.bits.iter().map(|b| b.load(Ordering::Relaxed));
        out.extend([
            (
                "weighted pagerank",
                Hash::default()
                    .f64s(ranks.unwrap().values.iter().copied())
                    .hex(),
            ),
            (
                "weighted eigenvector",
                Hash::default().f64s(eigen.values.iter().copied()).hex(),
            ),
            ("sssp", Hash::default().u64s(bits).hex()),
        ]);
        out
    })
}

/// The hash of what two deterministic runs of Leiden must share: the membership, every
/// level, the count and the modularity's bits.
#[must_use]
pub fn communities_hash(c: &Communities) -> String {
    let mut h = Hash::default();
    h.u32s(c.membership.iter().copied());
    for level in &c.levels {
        h.u64s([level.len() as u64]).u32s(level.iter().copied());
    }
    h.u32s([c.count]).f64s([c.modularity]);
    h.hex()
}

/// Leiden with `options` on `source`.
#[must_use]
pub fn communities<S: Source>(source: &S, settings: Settings, options: &Options) -> Communities {
    leiden(source, &Memory::unlimited(), settings, options).unwrap()
}
