// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Helpers that more than one test of the crate uses.

#![allow(clippy::cast_possible_truncation, reason = "test graphs are small")]

use everygraph_oracle::projection::{Adjacency as Analytic, Projection};

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
