// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The in-adjacency of a directed graph as a graph file stores it: derived from the
//! out-adjacency by `everygraph-ingest` (#38).

#![allow(dead_code, reason = "each test uses part of it")]

use std::num::NonZeroUsize;

use everygraph_core::{ByteSource, Memory, MemorySource, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Encoded, encode};
use everygraph_format::container::{Container, Kind, Provenance, Writer};
use everygraph_ingest::build::{
    IN_DATA, IN_INDEX, OUT_DATA, OUT_INDEX, PROVENANCE, Silent, add_in_adjacency,
};
use everygraph_ingest::ids::MemoryScratch;

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
