// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Helpers that more than one test of the crate uses: the in-adjacency of a directed
//! graph as a graph file stores it, derived from the out-adjacency by
//! `everygraph-ingest` (#38), and an analytic adjacency with its kept loops written in.

#![allow(dead_code, reason = "each test uses part of it")]

use std::num::NonZeroUsize;

use everygraph_core::{ByteSource, Memory, MemorySource, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Encoded, encode};
use everygraph_format::container::{Container, Kind, Provenance, Writer};
use everygraph_ingest::build::{
    IN_DATA, IN_INDEX, OUT_DATA, OUT_INDEX, PROVENANCE, Silent, add_in_adjacency,
};
use everygraph_ingest::ids::MemoryScratch;
use everygraph_oracle::projection::{Adjacency as Analytic, Projection};

/// The in-adjacency that `everygraph-ingest` adds to a directed graph whose
/// out-adjacency is `out`, built on 3 threads.
pub fn in_adjacency(out: &Encoded) -> Encoded {
    let mut w = Writer::create(MemorySource::new(Vec::new())).unwrap();
    let record = Provenance {
        parameters: [("directed".into(), "true".into())].into(),
        ..Provenance::default()
    };
    w.add_provenance(PROVENANCE, &record).unwrap();
    w.add_section(OUT_INDEX, Kind::ADJACENCY_INDEX, &out.index, None)
        .unwrap();
    w.add_section(OUT_DATA, Kind::ADJACENCY_DATA, &out.data, None)
        .unwrap();
    w.commit().unwrap();
    let memory = Memory::unlimited();
    let threads = NonZeroUsize::new(3).unwrap();
    let (sink, added) = par::with_threads(threads, || {
        add_in_adjacency(w.into_sink(), &memory, &MemoryScratch, threads, &Silent)
    })
    .unwrap()
    .unwrap();
    assert!(added);
    let file = Container::open(sink).unwrap();
    let section = |name| file.section(name).unwrap().as_slice().unwrap().to_vec();
    Encoded {
        index: section(IN_INDEX),
        data: section(IN_DATA),
    }
}

/// The lists of `e` as a raw CSR.
pub fn decoded(e: &Encoded) -> (Vec<u64>, Vec<u32>) {
    let index = AdjacencyIndex::parse(&e.index).unwrap();
    let nodes = usize::try_from(index.nodes()).unwrap();
    let mut offsets = vec![0; nodes + 1];
    let mut targets = vec![0; usize::try_from(index.entries()).unwrap()];
    let adjacency = Adjacency::new(&index, &e.data).unwrap();
    adjacency.decode_all(&mut offsets, &mut targets).unwrap();
    (offsets, targets)
}

/// The in-lists of the out-lists `offsets` and `targets`, through [`in_adjacency`].
pub fn in_lists(offsets: &[u64], targets: &[u32]) -> (Vec<u64>, Vec<u32>) {
    decoded(&in_adjacency(&encode(offsets, targets).unwrap()))
}

/// One adjacency of the analytic graph with its kept loops written in, and its
/// per-entry columns: the adjacency as the engine stores it, where a kept loop is the
/// entry `v` in the list of `v`.
pub struct Lists {
    pub offsets: Vec<u64>,
    pub targets: Vec<u32>,
    /// The pair's summed weight; a loop's weight.
    pub weight: Vec<f64>,
    /// The pair's smallest event weight; 0 for a loop.
    pub cost: Vec<f64>,
}

#[allow(clippy::cast_possible_truncation, reason = "test graphs are small")]
pub fn with_loops(adj: &Analytic, p: &Projection) -> Lists {
    let mut l = Lists {
        offsets: vec![0],
        targets: Vec::new(),
        weight: Vec::new(),
        cost: Vec::new(),
    };
    for v in 0..adj.n() {
        let r = adj.range(v);
        let at = r.start + adj.list(v).partition_point(|&u| (u as usize) < v);
        let entry = |e: usize| (adj.targets[e], adj.weight_sum[e], adj.weight_min[e]);
        let lower = (r.start..at).map(entry);
        let kept = (p.kept_loops(v) == 1).then_some((v as u32, p.loop_weight[v], 0.0));
        for (t, w, c) in lower.chain(kept).chain((at..r.end).map(entry)) {
            l.targets.push(t);
            l.weight.push(w);
            l.cost.push(c);
        }
        l.offsets.push(l.targets.len() as u64);
    }
    l
}
