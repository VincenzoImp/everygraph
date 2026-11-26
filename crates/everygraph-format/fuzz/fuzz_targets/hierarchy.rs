// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The check of a stored community hierarchy on untrusted bytes (issue #62): what
//! `everygraph verify --deep` runs after the format's own checks. Nothing may panic.
//!
//! The first byte picks the mode:
//! - 0: a whole file, opened and checked;
//! - 1: a decision seed: the first half of the input is a small graph, whose Leiden
//!   hierarchy is stored in its graph file; the second half damages that file. The
//!   undamaged file must check intact.

#![no_main]

use everygraph_community::hierarchy::{self, store};
use everygraph_community::{Options, leiden};
use everygraph_core::{Memory, MemorySource};
use everygraph_format::adjacency::encode;
use everygraph_format::container::{Container, Kind, Provenance, Writer};
use everygraph_format::verify::{Depth, Report, verify};
use everygraph_kernels::edge_map::{Csr, Settings};
use libfuzzer_sys::fuzz_target;

fn checked(bytes: &[u8]) -> Option<Report> {
    let source = MemorySource::new(bytes);
    let memory = Memory::unlimited();
    let mut report = verify(&source, Depth::Deep, &memory).expect("memory does not fail");
    let file = Container::open(&source).ok()?;
    hierarchy::check(&file, &memory, &mut report).expect("memory does not fail");
    Some(report)
}

/// A graph of up to 32 nodes from `bytes`, two bytes per pair, and the file holding it
/// with the hierarchy of its communities.
fn build(bytes: &[u8]) -> Option<Vec<u8>> {
    let (&n, pairs) = bytes.split_first()?;
    let n = 2 + u32::from(n % 31);
    let mut lists = vec![Vec::new(); n as usize];
    for p in pairs.chunks_exact(2) {
        let (u, v) = (u32::from(p[0]) % n, u32::from(p[1]) % n);
        lists[u as usize].push(v);
        if u != v {
            lists[v as usize].push(u);
        }
    }
    let (mut offsets, mut targets) = (vec![0_u64], Vec::new());
    for mut l in lists {
        l.sort_unstable();
        l.dedup();
        targets.extend(l);
        offsets.push(targets.len() as u64);
    }
    let encoded = encode(&offsets, &targets).ok()?;
    let mut w = Writer::create(MemorySource::new(Vec::new())).ok()?;
    let record = Provenance {
        parameters: [("directed", "false"), ("self_loops", "keep")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into_iter()
            .collect(),
        ..Provenance::default()
    };
    w.add_provenance("provenance/ingest", &record).ok()?;
    w.add_section("adjacency.out.index", Kind::ADJACENCY_INDEX, &encoded.index, Some("provenance/ingest"))
        .ok()?;
    w.add_section("adjacency.out.data", Kind::ADJACENCY_DATA, &encoded.data, Some("provenance/ingest"))
        .ok()?;
    let csr = Csr::new(&offsets, &targets).ok()?;
    let memory = Memory::unlimited();
    let options = Options::default();
    let c = leiden(&csr, &memory, Settings::default(), &options).expect("a graph");
    let key = hierarchy::key(w.graph_identity(), &options).expect("a key");
    store(&mut w, &key, &Provenance::default(), &csr, &memory, Settings::default(), &c)
        .expect("the hierarchy is stored");
    w.commit().expect("commit");
    Some(w.into_sink().into_inner())
}

fn built_then_damaged(bytes: &[u8]) {
    let (graph, damage) = bytes.split_at(bytes.len() / 2);
    let Some(mut file) = build(graph) else {
        return;
    };
    let clean = checked(&file).expect("the file opens");
    assert!(clean.is_intact(), "{:?}", clean.problems);
    for hit in damage.chunks_exact(3) {
        let at = usize::from(u16::from_le_bytes([hit[0], hit[1]])) * 7 % file.len();
        file[at] ^= hit[2];
    }
    checked(&file);
}

fuzz_target!(|bytes: &[u8]| {
    let Some((&mode, rest)) = bytes.split_first() else {
        return;
    };
    if mode % 2 == 0 {
        checked(rest);
    } else {
        built_then_damaged(rest);
    }
});
