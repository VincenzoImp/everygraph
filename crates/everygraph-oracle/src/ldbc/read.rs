// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reading a Graphalytics data set: the properties, the vertex and edge files, and the
//! reference outputs (specification § 2.2.2, § 2.4).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::{Algorithm, Values};
use crate::{ix, to_u64, to_usize};

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// A data set directory: `<g>.properties` and the files it names.
#[derive(Clone, Debug)]
pub struct Dataset {
    /// The graph's name, `<g>`.
    pub name: String,
    /// The directory holding its files.
    pub dir: PathBuf,
    /// The `.properties` entries, with the `graph.<g>.` prefix removed.
    pub properties: BTreeMap<String, String>,
}

impl Dataset {
    /// Opens the data set in `dir`, which holds exactly one `.properties` file.
    ///
    /// # Errors
    ///
    /// If the directory cannot be read, has no or several `.properties` files, or one
    /// cannot be read.
    pub fn open(dir: &Path) -> io::Result<Self> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "properties") {
                found.push(path);
            }
        }
        let [path] = found.as_slice() else {
            return Err(bad(format!(
                "{}: expected one .properties file, found {}",
                dir.display(),
                found.len()
            )));
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| bad("properties file name is not UTF-8"))?
            .to_owned();
        let prefix = format!("graph.{name}.");
        let mut properties = BTreeMap::new();
        for line in std::fs::read_to_string(path)?.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let k = k.strip_prefix(&prefix).unwrap_or(k);
                properties.insert(k.to_owned(), v.trim().to_owned());
            }
        }
        Ok(Self {
            name,
            dir: dir.to_owned(),
            properties,
        })
    }

    /// A property by its key without the prefix (`"pr.num-iterations"`).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(String::as_str)
    }

    fn parsed<T: std::str::FromStr>(&self, key: &str) -> io::Result<T> {
        let v = self
            .get(key)
            .ok_or_else(|| bad(format!("{}: no property {key}", self.name)))?;
        v.parse()
            .map_err(|_| bad(format!("{}: bad value {v:?} for {key}", self.name)))
    }

    /// Whether the graph is directed.
    ///
    /// # Errors
    ///
    /// If the property is missing or not a boolean.
    pub fn directed(&self) -> io::Result<bool> {
        self.parsed("directed")
    }

    /// Whether the edges carry a weight (an edge property is declared).
    #[must_use]
    pub fn weighted(&self) -> bool {
        self.get("edge-properties.names").is_some()
    }

    /// The vertex and edge counts the properties declare.
    ///
    /// # Errors
    ///
    /// If either is missing or not an integer.
    pub fn declared_size(&self) -> io::Result<(u64, u64)> {
        Ok((self.parsed("meta.vertices")?, self.parsed("meta.edges")?))
    }

    /// The algorithms the data set lists, with a reference output file present.
    #[must_use]
    pub fn algorithms(&self) -> Vec<Algorithm> {
        let listed = self.get("algorithms").unwrap_or("");
        listed
            .split(',')
            .filter_map(|a| Algorithm::parse(a.trim()))
            .filter(|&a| self.reference_path(a).exists())
            .collect()
    }

    /// The source vertex id of BFS or SSSP.
    ///
    /// # Errors
    ///
    /// If the property is missing or not an integer.
    pub fn source(&self, a: Algorithm) -> io::Result<u64> {
        self.parsed(&format!("{}.source-vertex", a.name()))
    }

    /// PageRank's damping factor.
    ///
    /// # Errors
    ///
    /// If the property is missing or not a number.
    pub fn damping(&self) -> io::Result<f64> {
        self.parsed("pr.damping-factor")
    }

    /// The iteration count of PageRank or CDLP.
    ///
    /// # Errors
    ///
    /// If the property is missing or not an integer.
    pub fn iterations(&self, a: Algorithm) -> io::Result<u32> {
        match a {
            Algorithm::Pr => self.parsed("pr.num-iterations"),
            Algorithm::Cdlp => self.parsed("cdlp.max-iterations"),
            _ => Err(bad(format!("{} has no iterations", a.name()))),
        }
    }

    /// The reference output file of `a`.
    #[must_use]
    pub fn reference_path(&self, a: Algorithm) -> PathBuf {
        self.dir.join(format!("{}-{}", self.name, a.suffix()))
    }

    /// `<g>.v` or `<g>.e`. The names every archive uses, not the `vertex-file` and
    /// `edge-file` properties: test-sssp-undirected (archive of 2023-03-05) names its
    /// vertex file as its edge file. [`Dataset::notes`] reports such a property.
    fn file(&self, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", self.name))
    }

    /// Properties that disagree with the files read.
    #[must_use]
    pub fn notes(&self) -> Vec<String> {
        [("vertex-file", "v"), ("edge-file", "e")]
            .into_iter()
            .filter_map(|(key, ext)| {
                let want = format!("{}.{ext}", self.name);
                self.get(key)
                    .filter(|&v| v != want)
                    .map(|v| format!("{key} = {v} in the properties; {want} is read"))
            })
            .collect()
    }

    /// Reads the graph.
    ///
    /// # Errors
    ///
    /// If a file cannot be read, or the graph breaks a rule of the specification: an
    /// unknown or repeated vertex, a self-loop, a repeated edge, a weight that is
    /// negative or not finite.
    pub fn load(&self) -> io::Result<LdbcGraph> {
        let ids = Ids::read(&self.file("v"))?;
        read_edges(&self.file("e"), ids, self.directed()?, self.weighted())
    }

    /// Reads the reference output of `a`.
    ///
    /// # Errors
    ///
    /// If the file cannot be read, a line does not parse, or a vertex is missing,
    /// unknown or listed twice.
    pub fn reference(&self, a: Algorithm, ids: &Ids) -> io::Result<Values> {
        read_values(&self.reference_path(a), a, ids)
    }
}

/// Calls `f` on every line of a file, without its line end.
fn for_each_line(path: &Path, mut f: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut buf = vec![0u8; 1 << 23];
    let mut keep = 0;
    loop {
        let got = file.read(&mut buf[keep..])?;
        let end = keep + got;
        let mut start = 0;
        while let Some(p) = buf[start..end].iter().position(|&b| b == b'\n') {
            f(trim_cr(&buf[start..start + p]))?;
            start += p + 1;
        }
        if got == 0 {
            if start < end {
                f(trim_cr(&buf[start..end]))?;
            }
            return Ok(());
        }
        buf.copy_within(start..end, 0);
        keep = end - start;
        if keep == buf.len() {
            buf.resize(buf.len() * 2, 0);
        }
    }
}

fn trim_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn fields(line: &[u8]) -> impl Iterator<Item = &[u8]> {
    line.split(|&b| b == b' ' || b == b'\t')
        .filter(|f| !f.is_empty())
}

fn parse_u64(f: &[u8]) -> Option<u64> {
    if f.is_empty() || f.len() > 20 {
        return None;
    }
    f.iter().try_fold(0u64, |acc, &b| {
        b.is_ascii_digit()
            .then(|| acc.checked_mul(10)?.checked_add(u64::from(b - b'0')))
            .flatten()
    })
}

fn parse_f64(f: &[u8]) -> Option<f64> {
    std::str::from_utf8(f).ok()?.parse().ok()
}

fn line_text(line: &[u8]) -> String {
    String::from_utf8_lossy(line).into_owned()
}

/// The vertex ids, sorted, and the map from id to dense index.
#[derive(Clone, Debug)]
pub struct Ids {
    /// The id of each dense index, increasing.
    pub ids: Vec<u64>,
    /// `table[id]` = index, or `u32::MAX`; used when the ids are dense enough.
    table: Option<Vec<u32>>,
}

impl Ids {
    /// Builds the map from distinct ids in any order.
    ///
    /// # Errors
    ///
    /// If an id is repeated, or there are `u32::MAX` ids or more.
    pub fn new(mut ids: Vec<u64>) -> io::Result<Self> {
        ids.sort_unstable();
        if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(bad(format!("vertex {} is listed twice", w[0])));
        }
        if u32::try_from(ids.len()).is_err() {
            return Err(bad("2^32 vertices or more"));
        }
        // A direct table when it costs at most 8 bytes per vertex; else binary search.
        let max = ids.last().copied().unwrap_or(0);
        let table = (max < 2 * to_u64(ids.len()) + 1024).then(|| {
            let mut t = vec![u32::MAX; to_usize(max) + 1];
            for (i, &id) in (0u32..).zip(&ids) {
                t[to_usize(id)] = i;
            }
            t
        });
        Ok(Self { ids, table })
    }

    fn read(path: &Path) -> io::Result<Self> {
        let mut ids = Vec::new();
        for_each_line(path, |line| {
            if let Some(f) = fields(line).next() {
                ids.push(
                    parse_u64(f)
                        .ok_or_else(|| bad(format!("bad vertex line {:?}", line_text(line))))?,
                );
            }
            Ok(())
        })?;
        Self::new(ids)
    }

    /// Number of vertices.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True if there are no vertices.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The dense index of `id`.
    #[must_use]
    pub fn index(&self, id: u64) -> Option<u32> {
        match &self.table {
            Some(t) => usize::try_from(id)
                .ok()
                .and_then(|i| t.get(i).copied())
                .filter(|&i| i != u32::MAX),
            None => self
                .ids
                .binary_search(&id)
                .ok()
                .and_then(|i| u32::try_from(i).ok()),
        }
    }

    fn index_of(&self, f: Option<&[u8]>, line: &[u8]) -> io::Result<u32> {
        f.and_then(parse_u64)
            .and_then(|id| self.index(id))
            .ok_or_else(|| bad(format!("unknown vertex in line {:?}", line_text(line))))
    }
}

/// Adjacency lists, each sorted by target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Csr {
    /// `n + 1` offsets into `targets`.
    pub offsets: Vec<u64>,
    /// Targets, as dense indices.
    pub targets: Vec<u32>,
    /// One weight per entry, if the graph is weighted.
    pub weights: Option<Vec<f64>>,
}

impl Csr {
    /// A borrowed view.
    #[must_use]
    pub fn lists(&self) -> Lists<'_> {
        Lists {
            offsets: &self.offsets,
            targets: &self.targets,
            weights: self.weights.as_deref(),
        }
    }
}

/// A borrowed CSR: what [`super::spec`] runs on, so that callers may hand it
/// memory-mapped slices.
#[derive(Clone, Copy, Debug)]
pub struct Lists<'a> {
    /// `n + 1` offsets.
    pub offsets: &'a [u64],
    /// Targets, sorted within each list.
    pub targets: &'a [u32],
    /// One weight per entry, if any.
    pub weights: Option<&'a [f64]>,
}

impl Lists<'_> {
    /// Number of vertices.
    #[must_use]
    pub fn n(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// Entry positions of `v`'s list.
    #[must_use]
    pub fn range(&self, v: usize) -> std::ops::Range<usize> {
        to_usize(self.offsets[v])..to_usize(self.offsets[v + 1])
    }

    /// The list of `v`.
    #[must_use]
    pub fn list(&self, v: usize) -> &[u32] {
        &self.targets[self.range(v)]
    }

    /// The length of `v`'s list.
    #[must_use]
    pub fn degree(&self, v: usize) -> usize {
        self.range(v).len()
    }
}

/// A Graphalytics graph with dense vertex indices.
#[derive(Clone, Debug)]
pub struct LdbcGraph {
    /// Index ↔ id.
    pub ids: Ids,
    /// Directed or not.
    pub directed: bool,
    /// Out-lists; for an undirected graph, the symmetric lists (each edge both ways).
    pub out: Csr,
    /// In-lists of a directed graph, without weights.
    pub inn: Option<Csr>,
    /// The rank of each vertex in order of first appearance in the edge file; vertices
    /// in no edge follow, in id order. Used to show what CDLP gives when initial labels
    /// follow input order instead of id order.
    pub first_seen: Vec<u32>,
}

impl LdbcGraph {
    /// The in-lists: those of a directed graph, the symmetric lists otherwise.
    #[must_use]
    pub fn inn(&self) -> Lists<'_> {
        self.inn
            .as_ref()
            .map_or_else(|| self.out.lists(), Csr::lists)
    }

    /// Number of edges as the specification counts them (an undirected edge once).
    #[must_use]
    pub fn edges(&self) -> usize {
        if self.directed {
            self.out.targets.len()
        } else {
            self.out.targets.len() / 2
        }
    }
}

fn read_edges(path: &Path, ids: Ids, directed: bool, weighted: bool) -> io::Result<LdbcGraph> {
    let n = ids.len();
    let mut out_deg = vec![0u64; n + 1];
    let mut in_deg = if directed {
        vec![0u64; n + 1]
    } else {
        Vec::new()
    };
    let mut first_seen = vec![u32::MAX; n];
    let mut seen = 0u32;
    let mut note = |v: u32| {
        let slot = &mut first_seen[ix(v)];
        if *slot == u32::MAX {
            *slot = seen;
            seen += 1;
        }
    };
    for_each_line(path, |line| {
        let mut f = fields(line);
        let Some(first) = f.next() else {
            return Ok(());
        };
        let u = ids.index_of(Some(first), line)?;
        let v = ids.index_of(f.next(), line)?;
        if u == v {
            return Err(bad(format!("self-loop in line {:?}", line_text(line))));
        }
        note(u);
        note(v);
        out_deg[ix(u) + 1] += 1;
        if directed {
            in_deg[ix(v) + 1] += 1;
        } else {
            out_deg[ix(v) + 1] += 1;
        }
        Ok(())
    })?;
    for s in &mut first_seen {
        if *s == u32::MAX {
            *s = seen;
            seen += 1;
        }
    }
    let prefix = |d: &mut Vec<u64>| {
        for i in 0..n {
            d[i + 1] += d[i];
        }
    };
    prefix(&mut out_deg);
    let mut out = Csr {
        targets: vec![0; to_usize(out_deg[n])],
        weights: weighted.then(|| vec![0.0; to_usize(out_deg[n])]),
        offsets: out_deg,
    };
    let mut inn = directed.then(|| {
        prefix(&mut in_deg);
        Csr {
            targets: vec![0; to_usize(in_deg[n])],
            weights: None,
            offsets: in_deg,
        }
    });
    let mut out_at: Vec<u64> = out.offsets[..n].to_vec();
    let mut in_at: Vec<u64> = inn
        .as_ref()
        .map_or_else(Vec::new, |c| c.offsets[..n].to_vec());
    for_each_line(path, |line| {
        let mut f = fields(line);
        let Some(first) = f.next() else {
            return Ok(());
        };
        let u = ids.index_of(Some(first), line)?;
        let v = ids.index_of(f.next(), line)?;
        let w = if weighted {
            let w = f
                .next()
                .and_then(parse_f64)
                .filter(|w| w.is_finite() && *w >= 0.0);
            Some(w.ok_or_else(|| bad(format!("bad weight in line {:?}", line_text(line))))?)
        } else {
            None
        };
        let mut put = |a: u32, b: u32| {
            let at = to_usize(out_at[ix(a)]);
            out_at[ix(a)] += 1;
            out.targets[at] = b;
            if let (Some(ws), Some(w)) = (out.weights.as_mut(), w) {
                ws[at] = w;
            }
        };
        put(u, v);
        match inn.as_mut() {
            Some(c) => {
                let at = to_usize(in_at[ix(v)]);
                in_at[ix(v)] += 1;
                c.targets[at] = u;
            }
            None => put(v, u),
        }
        Ok(())
    })?;
    drop((out_at, in_at));
    sort_lists(&mut out, &ids)?;
    if let Some(c) = inn.as_mut() {
        sort_lists(c, &ids)?;
    }
    Ok(LdbcGraph {
        ids,
        directed,
        out,
        inn,
        first_seen,
    })
}

/// Sorts every list and refuses a repeated edge (specification § 2.2.1: every edge is
/// unique; an undirected edge listed both ways is repeated too).
fn sort_lists(c: &mut Csr, ids: &Ids) -> io::Result<()> {
    let n = c.offsets.len() - 1;
    let mut pairs: Vec<(u32, f64)> = Vec::new();
    for v in 0..n {
        let r = to_usize(c.offsets[v])..to_usize(c.offsets[v + 1]);
        match c.weights.as_mut() {
            None => c.targets[r.clone()].sort_unstable(),
            Some(ws) => {
                pairs.clear();
                pairs.extend(
                    c.targets[r.clone()]
                        .iter()
                        .copied()
                        .zip(ws[r.clone()].iter().copied()),
                );
                pairs.sort_unstable_by_key(|p| p.0);
                for (k, (t, w)) in pairs.iter().enumerate() {
                    c.targets[r.start + k] = *t;
                    ws[r.start + k] = *w;
                }
            }
        }
        if let Some(w) = c.targets[r].windows(2).find(|w| w[0] == w[1]) {
            return Err(bad(format!(
                "edge {} {} is listed twice",
                ids.ids[v],
                ids.ids[ix(w[0])]
            )));
        }
    }
    Ok(())
}

/// Symmetric lists of a directed graph with, per entry `v → u`, which arcs exist:
/// bit 0 for `v → u`, bit 1 for `u → v`. What WCC and LCC read on a directed graph.
#[must_use]
pub fn merge_directions(out: Lists<'_>, inn: Lists<'_>) -> (Csr, Vec<u8>) {
    let n = out.n();
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0u64);
    let mut targets = Vec::new();
    let mut dirs = Vec::new();
    for v in 0..n {
        let (a, b) = (out.list(v), inn.list(v));
        let (mut i, mut j) = (0, 0);
        while i < a.len() || j < b.len() {
            let x = a.get(i).copied().unwrap_or(u32::MAX);
            let y = b.get(j).copied().unwrap_or(u32::MAX);
            let t = x.min(y);
            let mut d = 0u8;
            if x == t {
                d |= 1;
                i += 1;
            }
            if y == t {
                d |= 2;
                j += 1;
            }
            targets.push(t);
            dirs.push(d);
        }
        offsets.push(to_u64(targets.len()));
    }
    (
        Csr {
            offsets,
            targets,
            weights: None,
        },
        dirs,
    )
}

fn read_values(path: &Path, a: Algorithm, ids: &Ids) -> io::Result<Values> {
    let n = ids.len();
    let ints = matches!(a, Algorithm::Bfs | Algorithm::Cdlp | Algorithm::Wcc);
    let mut iv = if ints { vec![0u64; n] } else { Vec::new() };
    let mut rv = if ints { Vec::new() } else { vec![0f64; n] };
    let mut seen = vec![false; n];
    for_each_line(path, |line| {
        let mut f = fields(line);
        let Some(first) = f.next() else {
            return Ok(());
        };
        let v = ix(ids.index_of(Some(first), line)?);
        if std::mem::replace(&mut seen[v], true) {
            return Err(bad(format!("vertex listed twice: {:?}", line_text(line))));
        }
        let value = f.next();
        let parsed = if ints {
            value.and_then(parse_u64).map(|x| iv[v] = x)
        } else {
            value.and_then(parse_f64).map(|x| rv[v] = x)
        };
        parsed.ok_or_else(|| bad(format!("bad value in line {:?}", line_text(line))))
    })?;
    if let Some(v) = seen.iter().position(|s| !s) {
        return Err(bad(format!(
            "{}: vertex {} has no value",
            path.display(),
            ids.ids[v]
        )));
    }
    Ok(if ints {
        Values::Ints(iv)
    } else {
        Values::Reals(rv)
    })
}
