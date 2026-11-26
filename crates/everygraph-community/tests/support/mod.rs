// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The graph generators of `tests/references.py`, and graphs and graph files, shared by
//! the tests of this crate.

#![allow(dead_code)]

use everygraph_community::hierarchy;
use everygraph_core::{Memory, MemorySource};
use everygraph_format::adjacency::encode;
use everygraph_format::container::{Container, Kind, Provenance, Writer};
use everygraph_format::verify::{Depth, Report, verify};
use everygraph_kernels::edge_map::Csr;

/// A graph as its node count and its pairs.
pub type Pairs = (u32, Vec<(u32, u32)>);

/// The generator of `tests/references.py`.
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A float in [0, 1) from the top 53 bits of a draw: `unit` of the script.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }
}

pub fn planted(blocks: u32, size: u32, p_in: u64, p_out: u64, seed: u64) -> Pairs {
    let mut rng = SplitMix64(seed);
    let n = blocks * size;
    let mut edges = Vec::new();
    for u in 0..n {
        for v in u + 1..n {
            let p = if u / size == v / size { p_in } else { p_out };
            if rng.next() % 1000 < p {
                edges.push((u, v));
            }
        }
    }
    (n, edges)
}

pub fn ring(cliques: u32, size: u32) -> Pairs {
    let mut edges = Vec::new();
    for c in 0..cliques {
        let base = c * size;
        for a in 0..size {
            for b in a + 1..size {
                edges.push((base + a, base + b));
            }
        }
        edges.push((base + size - 1, (c + 1) % cliques * size));
    }
    (cliques * size, edges)
}

pub fn karate() -> Pairs {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../oracle/golden/karate.tsv"
    );
    let text = std::fs::read_to_string(path).unwrap();
    let edges = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut it = l.split_whitespace().map(|x| x.parse::<u32>().unwrap());
            (it.next().unwrap(), it.next().unwrap())
        })
        .collect();
    (34, edges)
}

/// `blocks` blocks of `size` nodes: each node draws `d_in` neighbours in its block and
/// `d_out` anywhere (repeats and loops dropped).
pub fn sparse_planted(blocks: u32, size: u32, d_in: u32, d_out: u32, seed: u64) -> Pairs {
    let mut rng = SplitMix64(seed);
    let n = blocks * size;
    let mut edges = Vec::new();
    for u in 0..n {
        let block = u / size * size;
        for i in 0..d_in + d_out {
            let v = if i < d_in {
                block + (rng.next() % u64::from(size)) as u32
            } else {
                (rng.next() % u64::from(n)) as u32
            };
            if v != u {
                edges.push((u, v));
            }
        }
    }
    (n, edges)
}

/// A weighted graph as its node count, its edges (arcs when directed) and their
/// weights.
pub type Weighted = (u32, Vec<(u32, u32)>, Vec<f64>);

/// `weighted_planted` of the script.
pub fn weighted_planted(
    blocks: u32,
    size: u32,
    p_in: u64,
    p_out: u64,
    seed: u64,
    directed: bool,
) -> Weighted {
    let mut rng = SplitMix64(seed);
    let n = blocks * size;
    let (mut edges, mut weights) = (Vec::new(), Vec::new());
    for u in 0..n {
        let first = if directed { 0 } else { u + 1 };
        for v in first..n {
            if u == v {
                continue;
            }
            let inside = u / size == v / size;
            if rng.next() % 1000 < if inside { p_in } else { p_out } {
                edges.push((u, v));
                weights.push(if inside {
                    1.0 + 2.0 * rng.unit()
                } else {
                    0.1 + 0.4 * rng.unit()
                });
            }
        }
    }
    for v in (0..n).step_by(7) {
        edges.push((v, v));
        weights.push(0.5 + rng.unit());
    }
    (n, edges, weights)
}

/// `weighted` of the script.
pub fn weighted((n, edges): Pairs, seed: u64) -> Weighted {
    let mut rng = SplitMix64(seed);
    let weights = edges.iter().map(|_| 0.5 + 4.0 * rng.unit()).collect();
    (n, edges, weights)
}

/// Lists of a graph with a weight per entry: each pair in both lists, a loop once, in
/// increasing order. Of a directed graph, the lists of its undirected projection, and
/// per entry of `v` in the list of `u` the weight of the arc `(u, v)` and of `(v, u)`
/// (0 when absent; a loop's in both).
pub struct Lists {
    pub offsets: Vec<u64>,
    pub targets: Vec<u32>,
    pub out: Vec<f64>,
    pub inn: Vec<f64>,
}

impl Lists {
    /// Of a graph whose edges are distinct, undirected unless `directed`. Undirected,
    /// `out` holds the pair's weight and `inn` is empty.
    pub fn new((n, edges, weights): &Weighted, directed: bool) -> Self {
        let mut entries: Vec<(u32, u32, f64, f64)> = Vec::new();
        for (&(u, v), &w) in edges.iter().zip(weights) {
            if u == v {
                entries.push((u, u, w, w));
            } else {
                entries.push((u, v, w, 0.0));
                entries.push((v, u, 0.0, w));
            }
        }
        entries.sort_by_key(|e| (e.0, e.1));
        let mut merged: Vec<(u32, u32, f64, f64)> = Vec::new();
        for e in entries {
            match merged.last_mut() {
                Some(last) if (last.0, last.1) == (e.0, e.1) => {
                    last.2 += e.2;
                    last.3 += e.3;
                }
                _ => merged.push(e),
            }
        }
        let mut offsets = vec![0_u64; *n as usize + 1];
        for e in &merged {
            offsets[e.0 as usize + 1] += 1;
        }
        for v in 0..*n as usize {
            offsets[v + 1] += offsets[v];
        }
        let pair = |e: &(u32, u32, f64, f64)| if e.0 == e.1 { e.2 } else { e.2 + e.3 };
        Self {
            offsets,
            targets: merged.iter().map(|e| e.1).collect(),
            out: merged
                .iter()
                .map(|e| if directed { e.2 } else { pair(e) })
                .collect(),
            inn: if directed {
                merged.iter().map(|e| e.3).collect()
            } else {
                Vec::new()
            },
        }
    }

    pub fn nodes(&self) -> u32 {
        (self.offsets.len() - 1) as u32
    }
}

/// Nested planted blocks: `sizes[0]` nodes per finest block, `sizes[1]` finest blocks
/// per block of the next level, and so on. Each node draws `degrees[k]` neighbours in
/// its block of level `k`, and `degrees[sizes.len()]` in the whole graph.
pub fn nested(sizes: &[u32], degrees: &[u32], seed: u64) -> (u32, Vec<(u32, u32)>) {
    let mut rng = SplitMix64(seed);
    let n: u32 = sizes.iter().product();
    let mut pairs = Vec::new();
    for u in 0..n {
        let mut span = 1;
        for (k, &d) in degrees.iter().enumerate() {
            span = if k < sizes.len() { span * sizes[k] } else { n };
            let base = u / span * span;
            for _ in 0..d {
                let v = base + (rng.next() % u64::from(span)) as u32;
                if v != u {
                    pairs.push((u, v));
                }
            }
        }
    }
    (n, pairs)
}

pub type Mem = MemorySource<Vec<u8>>;

/// Lists of an unweighted undirected graph: each pair in both lists, a kept loop once.
pub struct Plain {
    pub offsets: Vec<u64>,
    pub targets: Vec<u32>,
}

impl Plain {
    pub fn from_pairs(n: u32, pairs: &[(u32, u32)]) -> Self {
        let mut lists = vec![Vec::new(); n as usize];
        for &(u, v) in pairs {
            lists[u as usize].push(v);
            if u != v {
                lists[v as usize].push(u);
            }
        }
        let mut offsets = vec![0];
        let mut targets = Vec::new();
        for mut l in lists {
            l.sort_unstable();
            l.dedup();
            targets.extend(l);
            offsets.push(targets.len() as u64);
        }
        Self { offsets, targets }
    }

    pub fn nodes(&self) -> u32 {
        (self.offsets.len() - 1) as u32
    }

    pub fn list(&self, v: u32) -> &[u32] {
        &self.targets[self.offsets[v as usize] as usize..self.offsets[v as usize + 1] as usize]
    }

    pub fn csr(&self) -> Csr<'_> {
        Csr::new(&self.offsets, &self.targets).unwrap()
    }

    /// The subgraph induced by `nodes` (increasing), renumbered in their order.
    pub fn induced(&self, nodes: &[u32]) -> Self {
        let place = |v: u32| nodes.binary_search(&v).ok();
        let mut offsets = vec![0];
        let mut targets = Vec::new();
        for &v in nodes {
            targets.extend(
                self.list(v)
                    .iter()
                    .filter_map(|&w| place(w).map(|i| i as u32)),
            );
            offsets.push(targets.len() as u64);
        }
        Self { offsets, targets }
    }
}

/// A graph file holding `lists` as its undirected adjacency, loops kept.
pub fn graph_file(lists: &Plain) -> Writer<Mem> {
    let encoded = encode(&lists.offsets, &lists.targets).unwrap();
    let mut w = Writer::create(Mem::default()).unwrap();
    let record = Provenance {
        engine: "everygraph test".into(),
        operation: "ingest".into(),
        parameters: [("directed", "false"), ("self_loops", "keep")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        ..Provenance::default()
    };
    w.add_provenance("provenance/ingest", &record).unwrap();
    for (name, kind, bytes) in [
        ("adjacency.out.index", Kind::ADJACENCY_INDEX, &encoded.index),
        ("adjacency.out.data", Kind::ADJACENCY_DATA, &encoded.data),
    ] {
        w.add_section(name, kind, bytes, Some("provenance/ingest"))
            .unwrap();
    }
    w.commit().unwrap();
    w
}

pub fn about() -> Provenance {
    Provenance {
        engine: "everygraph test".into(),
        ..Provenance::default()
    }
}

/// What the strictest readers find in `bytes`.
pub fn checked(bytes: &[u8]) -> Report {
    let source = MemorySource::new(bytes);
    let memory = Memory::unlimited();
    let mut report = verify(&source, Depth::Deep, &memory).unwrap();
    hierarchy::check(&Container::open(&source).unwrap(), &memory, &mut report).unwrap();
    report
}
