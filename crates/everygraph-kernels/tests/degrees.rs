// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The `weighted` column (#50) against the oracle's analytic graph: the strength in the
//! undirected projection, a kept loop of weight `w` adding `2w`
//! (`docs/spec/graph-semantics.md` § 3), at 1 and 4 threads, with the identities
//! `Σ degree = 2m` and, for a directed graph, out- plus in-strength.

#![forbid(unsafe_code)]
#![allow(clippy::disallowed_methods, clippy::cast_possible_truncation)]

use std::num::NonZeroUsize;

use everygraph_core::{Memory, par};
use everygraph_kernels::degrees::{Loop, degrees, strengths};
use everygraph_kernels::edge_map::{Csr, Plan, Settings};
use everygraph_oracle::projection::{Adjacency, Projection};
use everygraph_oracle::strategies::arb_graph;
use proptest::prelude::*;

/// Offsets, targets and weights of `adj`, with each node's kept loop as its own entry.
fn with_loops(adj: &Adjacency, p: &Projection) -> (Vec<u64>, Vec<u32>, Vec<f64>) {
    let (mut offsets, mut targets, mut weights) = (Vec::from([0]), Vec::new(), Vec::new());
    for v in 0..adj.n() {
        let mut row: Vec<(u32, f64)> = adj
            .range(v)
            .map(|e| (adj.targets[e], adj.weight_sum[e]))
            .collect();
        if p.keep_loops && p.loops[v] > 0 {
            row.push((v as u32, p.loop_weight[v]));
            row.sort_by_key(|e| e.0);
        }
        targets.extend(row.iter().map(|e| e.0));
        weights.extend(row.iter().map(|e| e.1));
        offsets.push(targets.len() as u64);
    }
    (offsets, targets, weights)
}

fn run(lists: &(Vec<u64>, Vec<u32>, Vec<f64>), own: Loop, threads: usize) -> (Vec<u32>, Vec<f64>) {
    let csr = Csr::new(&lists.0, &lists.1).unwrap();
    let memory = Memory::unlimited();
    par::with_threads(NonZeroUsize::new(threads).unwrap(), || {
        Plan::with(
            &csr,
            &memory,
            Settings {
                chunk_entries: 8,
                ..Settings::default()
            },
            |plan| {
                let d = degrees(&plan, &memory, own).unwrap();
                let s = strengths(&plan, &memory, &lists.2, own).unwrap();
                (d.iter().copied().collect(), s.iter().copied().collect())
            },
        )
    })
    .unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 64 } else { 256 }))]

    #[test]
    fn strengths_are_the_projection_with_loops_doubled(g in arb_graph(60)) {
        let p = Projection::new(&g);
        let und = with_loops(&p.undirected, &p);
        let (degree, strength) = run(&und, Loop::Twice, 1);
        prop_assert_eq!(run(&und, Loop::Twice, 4), (degree.clone(), strength.clone()));
        let loops = (0..p.len()).filter(|&v| p.keep_loops && p.loops[v] > 0).count();
        prop_assert_eq!(
            degree.iter().map(|&d| u64::from(d)).sum::<u64>(),
            p.undirected.entries() as u64 + 2 * loops as u64
        );
        for v in 0..p.len() {
            let pairs: f64 = p.undirected.range(v).map(|e| p.undirected.weight_sum[e]).sum();
            let want = pairs + 2.0 * p.loop_weight[v];
            let close = (strength[v] - want).abs() <= 1e-12 * want.abs().max(1.0);
            prop_assert!(close, "node {}: {} against {}", v, strength[v], want);
        }
        if p.directed {
            let (out_d, out_s) = run(&with_loops(&p.out, &p), Loop::Once, 4);
            let (in_d, in_s) = run(&with_loops(&p.inn, &p), Loop::Once, 4);
            prop_assert_eq!(out_d.iter().sum::<u32>(), in_d.iter().sum::<u32>());
            for v in 0..p.len() {
                let close = (out_s[v] + in_s[v] - strength[v]).abs() <= 1e-9 * strength[v].abs().max(1.0);
                prop_assert!(close, "node {}: {} + {} against {}", v, out_s[v], in_s[v], strength[v]);
            }
        }
    }
}
