// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Graphs and graph files for the tests of the map tree.

#![allow(dead_code)]

use everygraph_community::hierarchy;
use everygraph_core::{Memory, MemorySource};
use everygraph_format::adjacency::encode;
use everygraph_format::container::{Container, Kind, Provenance, Writer};
use everygraph_format::verify::{Depth, Report, verify};
use everygraph_kernels::edge_map::Csr;

pub type Mem = MemorySource<Vec<u8>>;

/// Lists of an undirected graph: each pair in both lists, a kept loop once.
pub struct Lists {
    pub offsets: Vec<u64>,
    pub targets: Vec<u32>,
}

impl Lists {
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
pub fn graph_file(lists: &Lists) -> Writer<Mem> {
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

/// `cliques` cliques of `size` nodes in a ring, each joined to the next by one edge.
pub fn ring(cliques: u32, size: u32) -> (u32, Vec<(u32, u32)>) {
    let mut pairs = Vec::new();
    for c in 0..cliques {
        let base = c * size;
        for a in 0..size {
            for b in a + 1..size {
                pairs.push((base + a, base + b));
            }
        }
        pairs.push((base + size - 1, (c + 1) % cliques * size));
    }
    (cliques * size, pairs)
}
