// SPDX-License-Identifier: Apache-2.0 OR MIT

//! S6 (`docs/spec/graph-semantics.md` § 5.5) through the crate's own aggregation: on
//! every graph of the oracle's CI corpus, kept loops in the lists, the partition of each
//! level of a Leiden run has the same table and modularity, bit for bit, as its quotient
//! graph under singletons.

use everygraph_core::Memory;
use everygraph_kernels::edge_map::{Csr, Settings};
use everygraph_oracle::corpus::{Tier, entries};
use everygraph_oracle::projection::Projection;

use crate::aggregate::quotient_of;
use crate::{Options, leiden, table};

/// The undirected lists of `p`, a kept loop once in its node's list.
fn lists(p: &Projection) -> (Vec<u64>, Vec<u32>) {
    let sym = &p.undirected;
    let (mut offsets, mut targets) = (vec![0], Vec::new());
    for u in 0..sym.n() {
        let start = targets.len();
        targets.extend_from_slice(sym.list(u));
        if p.kept_loops(u) > 0 {
            targets.push(u32::try_from(u).unwrap());
            targets[start..].sort_unstable();
        }
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

#[test]
fn every_level_has_the_modularity_of_its_quotient() {
    let memory = Memory::unlimited();
    let s = Settings::default();
    let step = if cfg!(debug_assertions) { 5 } else { 1 };
    for e in entries(Tier::Ci).into_iter().step_by(step) {
        let (offsets, targets) = lists(&Projection::new(&e.graph));
        let csr = Csr::new(&offsets, &targets).unwrap();
        let c = leiden(&csr, &memory, s, &Options::default()).unwrap();
        let name = &e.graph.name;
        let mut prefix: Vec<u32> = (0..csr_nodes(&offsets)).collect();
        for level in &c.levels {
            for x in &mut prefix {
                *x = level[*x as usize];
            }
            let t = table(&csr, &memory, s, &prefix).unwrap();
            let tq = quotient_of(&csr, &memory, s, &prefix)
                .unwrap()
                .table(&memory, s)
                .unwrap();
            assert_eq!(
                t.modularity(1.0).to_bits(),
                tq.modularity(1.0).to_bits(),
                "{name}"
            );
            assert_eq!(&t.internal[..], &tq.internal[..], "{name}");
            assert_eq!(&t.volume[..], &tq.volume[..], "{name}");
        }
    }
}

fn csr_nodes(offsets: &[u64]) -> u32 {
    u32::try_from(offsets.len() - 1).unwrap()
}

/// The hierarchy of `c` on `csr` in a new container, encoded `window` entries at a time.
fn stored(csr: &Csr<'_>, c: &crate::Communities, window: u64) -> Vec<u8> {
    use everygraph_core::MemorySource;
    use everygraph_format::container::{Provenance, Writer};

    use crate::hierarchy::{key, write};

    let memory = Memory::unlimited();
    let mut w = Writer::create(MemorySource::new(Vec::new())).unwrap();
    let k = key(w.graph_identity(), &Options::default()).unwrap();
    let run = write::Run {
        source: csr,
        memory: &memory,
        settings: Settings::default(),
        window,
    };
    let maps: Vec<&[u32]> = c.levels.iter().map(|l| &l[..]).collect();
    write::store_with(&mut w, &k, &Provenance::default(), &run, &maps).unwrap();
    w.commit().unwrap();
    w.into_sink().into_inner()
}

/// The quotient graphs are encoded a window of rows at a time; windows of one row and
/// of three entries give the bytes of one window per level on every corpus graph (whose
/// default-window hierarchy `tests/hierarchy.rs` checks against the oracle).
#[test]
fn the_encoding_window_does_not_change_the_bytes() {
    let memory = Memory::unlimited();
    for e in entries(Tier::Ci) {
        let (offsets, targets) = lists(&Projection::new(&e.graph));
        let csr = Csr::new(&offsets, &targets).unwrap();
        let c = leiden(&csr, &memory, Settings::default(), &Options::default()).unwrap();
        if c.levels.is_empty() {
            continue;
        }
        let whole = stored(&csr, &c, u64::MAX);
        for window in [1, 3] {
            assert!(
                stored(&csr, &c, window) == whole,
                "{}: window {window}",
                e.graph.name
            );
        }
    }
}
