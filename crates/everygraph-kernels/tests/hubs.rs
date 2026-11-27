// SPDX-License-Identifier: Apache-2.0 OR MIT

//! k-core and SSSP on a graph of hubs, against the oracle's references: the paths that
//! only lists of more than 1,024 entries reach. The frontier driver cuts such a list
//! into parts that run in parallel (`Plan::push_with`), and k-core sends a node of more
//! than 16,384 entries to the next subround rather than to a local queue. On `Csr` and
//! on the compressed source, at 1, 3 and 8 threads.

#![forbid(unsafe_code)]
// Test graphs and their expected values are small buffers outside the budget.
#![allow(clippy::disallowed_methods)]
// Small generated graphs: casts cannot truncate; the distances are sums of integers.
#![allow(clippy::cast_possible_truncation, clippy::float_cmp)]

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::atomic::Ordering::Relaxed;

use everygraph_core::{Memory, par};
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, encode};
use everygraph_kernels::edge_map::{Compressed, Csr, Plan, Settings, Source};
use everygraph_kernels::kcore::kcore;
use everygraph_kernels::sssp::{Sssp, sssp};
use everygraph_oracle::projection::Adjacency as Analytic;
use everygraph_oracle::reference;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A G(n, m) of 2,000 nodes and 12,000 edges, and hubs joined to it: one of 20,000
/// leaves (k-core samples it), one of 18,000 leaves and one edge into the G(n, m)
/// (its degree drops to 1 while its leaves are peeled, above 16,384), and three of
/// 1,500, 4,000 and 12,000 leaves with a few edges each. Symmetric, no loops.
fn graph() -> (Vec<u64>, Vec<u32>) {
    let mut edges = BTreeSet::new();
    let mut s = 3;
    let g = 2000;
    while edges.len() < 12_000 {
        let (a, b) = ((splitmix(&mut s) % g) as u32, (splitmix(&mut s) % g) as u32);
        if a != b {
            edges.insert((a.min(b), a.max(b)));
        }
    }
    let mut next = g as u32;
    for (leaves, links) in [
        (20_000, 40),
        (18_000, 1),
        (1_500, 3),
        (4_000, 7),
        (12_000, 20),
    ] {
        let hub = next;
        next += 1;
        for _ in 0..leaves {
            edges.insert((hub, next));
            next += 1;
        }
        for k in 0..links {
            edges.insert(((k * 37 % g) as u32, hub));
        }
    }
    let mut lists: Vec<Vec<u32>> = (0..next).map(|_| Vec::new()).collect();
    for &(a, b) in &edges {
        lists[a as usize].push(b);
        lists[b as usize].push(a);
    }
    let (mut offsets, mut targets) = (vec![0], Vec::new());
    for mut l in lists {
        l.sort_unstable();
        targets.extend(l);
        offsets.push(targets.len() as u64);
    }
    (offsets, targets)
}

/// The cost of entry `e` of the pair `(a, b)`: an integer from 1 to 9, the same in both
/// directions.
fn cost(a: u32, b: u32) -> f64 {
    let mut s = u64::from(a.min(b)) << 32 | u64::from(a.max(b));
    f64::from(1 + (splitmix(&mut s) % 9) as u32)
}

/// What the kernels give on `source` at `threads` threads: core numbers and distances.
fn run<S: Source + Sync>(
    source: &S,
    costs: &[f64],
    from: u32,
    threads: usize,
) -> (Vec<u64>, Vec<Option<f64>>) {
    let memory = Memory::unlimited();
    par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
        Plan::with(source, &memory, Settings::default(), |plan| {
            let (core, _) = kcore(&plan, &memory).unwrap();
            let core = core.iter().map(|c| u64::from(c.load(Relaxed))).collect();
            let d = sssp(&plan, costs, from, &Sssp::default(), &memory).unwrap();
            let dist = (0..plan.nodes()).map(|v| d.get(v)).collect();
            (core, dist)
        })
    })
    .unwrap()
}

#[test]
fn kcore_and_sssp_on_hubs_match_the_references() {
    let (offsets, targets) = graph();
    let n = offsets.len() - 1;
    let costs: Vec<f64> = (0..n)
        .flat_map(|v| {
            let list = &targets[offsets[v] as usize..offsets[v + 1] as usize];
            list.iter().map(move |&u| cost(v as u32, u))
        })
        .collect();
    let analytic = Analytic {
        offsets: offsets.clone(),
        targets: targets.clone(),
        multiplicity: vec![1; targets.len()],
        weight_sum: costs.clone(),
        weight_min: costs.clone(),
    };
    let core = reference::core_numbers(&analytic);
    // The hub of 18,000 leaves: it reaches the deferral with a degree above 16,384.
    let from = 2001 + 20_000;
    let dist = reference::sssp(&analytic, from as usize).unwrap();
    assert!(
        core.iter().any(|&c| c > 1),
        "the G(n, m) has a core above the hubs"
    );
    let encoded = encode(&offsets, &targets).unwrap();
    let index = AdjacencyIndex::parse(&encoded.index).unwrap();
    let csr = Csr::new(&offsets, &targets).unwrap();
    let heap = Compressed::new(Adjacency::new(&index, &encoded.data).unwrap());
    for threads in [1, 3, 8] {
        for (name, got) in [
            ("csr", run(&csr, &costs, from, threads)),
            ("compressed", run(&heap, &costs, from, threads)),
        ] {
            assert_eq!(got.0, core, "{name}, {threads} threads: core numbers");
            assert_eq!(got.1, dist, "{name}, {threads} threads: distances");
        }
    }
}
