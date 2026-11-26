// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The six Graphalytics algorithms as specification v1.0.5 § 2.3 defines them:
//! sequential, written for obviousness, and fast enough for the data sets of the S class
//! (about 10^8 edges) in minutes. Vertex indices are in increasing id order
//! ([`super::read::Ids`]), so "smallest label" and "smallest id" agree.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use super::UNREACHABLE;
use super::read::Lists;
use crate::{ix, to_u64};

fn index(v: usize) -> u32 {
    u32::try_from(v).expect("fewer than 2^32 vertices")
}

/// BFS depth from `source` along `out` (§ 2.3.1).
#[must_use]
pub fn bfs(out: Lists<'_>, source: u32) -> Vec<u64> {
    let mut depth = vec![UNREACHABLE; out.n()];
    depth[ix(source)] = 0;
    let mut queue = VecDeque::from([ix(source)]);
    while let Some(u) = queue.pop_front() {
        for &v in out.list(u) {
            if depth[ix(v)] == UNREACHABLE {
                depth[ix(v)] = depth[u] + 1;
                queue.push_back(ix(v));
            }
        }
    }
    depth
}

/// PageRank for a fixed number of iterations (§ 2.3.2, equation 2.3): teleport
/// `(1 - d) / |V|`, and the sinks' mass of the previous iteration spread over all
/// vertices.
#[must_use]
pub fn pagerank(out: Lists<'_>, inn: Lists<'_>, damping: f64, iterations: u32) -> Vec<f64> {
    let n = out.n();
    if n == 0 {
        return Vec::new();
    }
    let nf = to_u64(n) as f64;
    let mut rank = vec![1.0 / nf; n];
    let mut share = vec![0.0; n];
    for _ in 0..iterations {
        let mut sinks = 0.0;
        for u in 0..n {
            let d = out.degree(u);
            if d == 0 {
                sinks += rank[u];
                share[u] = 0.0;
            } else {
                share[u] = rank[u] / to_u64(d) as f64;
            }
        }
        let base = (1.0 - damping) / nf + damping * sinks / nf;
        for (v, r) in rank.iter_mut().enumerate() {
            let s: f64 = inn.list(v).iter().map(|&u| share[ix(u)]).sum();
            *r = base + damping * s;
        }
    }
    rank
}

/// Weakly connected components over symmetric lists (§ 2.3.3): each vertex gets the
/// smallest index of its component.
#[must_use]
pub fn wcc(sym: Lists<'_>) -> Vec<u32> {
    let n = sym.n();
    let mut label = vec![u32::MAX; n];
    let mut queue = VecDeque::new();
    for s in 0..n {
        if label[s] != u32::MAX {
            continue;
        }
        label[s] = index(s);
        queue.push_back(s);
        while let Some(u) = queue.pop_front() {
            for &v in sym.list(u) {
                if label[ix(v)] == u32::MAX {
                    label[ix(v)] = index(s);
                    queue.push_back(ix(v));
                }
            }
        }
    }
    label
}

/// Synchronous label propagation (§ 2.3.4, equation 2.5). `initial` gives each vertex
/// its starting label, distinct per vertex; the specification uses the vertex id, which
/// is the identity here. A vertex takes the most frequent label among its in- and
/// out-neighbours (a neighbour on both sides counts twice), the smallest on a tie, and
/// keeps its label if it has no neighbours. `inn` is `None` on an undirected graph.
#[must_use]
pub fn cdlp(out: Lists<'_>, inn: Option<Lists<'_>>, initial: &[u32], iterations: u32) -> Vec<u32> {
    let n = out.n();
    let mut label = initial.to_vec();
    let mut next = vec![0u32; n];
    let mut seen: Vec<u32> = Vec::new();
    for _ in 0..iterations {
        for (v, slot) in next.iter_mut().enumerate() {
            seen.clear();
            seen.extend(out.list(v).iter().map(|&u| label[ix(u)]));
            if let Some(inn) = inn {
                seen.extend(inn.list(v).iter().map(|&u| label[ix(u)]));
            }
            seen.sort_unstable();
            let mut best = (0usize, label[v]);
            for run in seen.chunk_by(|a, b| a == b) {
                // Runs come in increasing label order: only a strictly larger count wins.
                if run.len() > best.0 {
                    best = (run.len(), run[0]);
                }
            }
            *slot = best.1;
        }
        std::mem::swap(&mut label, &mut next);
    }
    label
}

/// Local clustering coefficient (§ 2.3.5, equation 2.6). `sym` holds each vertex's
/// neighbours without direction, each once; `dirs[e]` says which arcs entry `e` stands
/// for (bit 0 `v → u`, bit 1 `u → v`; [`super::read::merge_directions`]); `None` means
/// an undirected graph, where every edge is both arcs.
///
/// The numerator counts ordered pairs `(u, w)` of neighbours with an arc `u → w`. Each
/// triangle `{v, u, w}` of the undirected projection adds the number of arcs between
/// `u` and `w` to `v`'s count. Triangles are listed once each, from their vertex of
/// lowest (degree, index) rank.
#[must_use]
pub fn lcc(sym: Lists<'_>, dirs: Option<&[u8]>) -> Vec<f64> {
    let n = sym.n();
    let arcs = |e: usize| dirs.map_or(2u8, |d| if d[e] == 3 { 2 } else { 1 });
    let before = |a: usize, b: usize| (sym.degree(a), a) < (sym.degree(b), b);
    // Forward lists: the neighbours of higher rank, in index order, with their arc count.
    let mut f_off = vec![0usize; n + 1];
    let mut f_target: Vec<u32> = Vec::new();
    let mut f_arcs: Vec<u8> = Vec::new();
    for v in 0..n {
        for e in sym.range(v) {
            if before(v, ix(sym.targets[e])) {
                f_target.push(sym.targets[e]);
                f_arcs.push(arcs(e));
            }
        }
        f_off[v + 1] = f_target.len();
    }
    let mut count = vec![0u64; n];
    // While v's forward list is marked, mark[w] is the arc count of v–w (0: unmarked).
    let mut mark = vec![0u8; n];
    for v in 0..n {
        let fv = f_off[v]..f_off[v + 1];
        for k in fv.clone() {
            mark[ix(f_target[k])] = f_arcs[k];
        }
        for k in fv.clone() {
            let u = ix(f_target[k]);
            for q in f_off[u]..f_off[u + 1] {
                let w = ix(f_target[q]);
                if mark[w] != 0 {
                    count[v] += u64::from(f_arcs[q]);
                    count[u] += u64::from(mark[w]);
                    count[w] += u64::from(f_arcs[k]);
                }
            }
        }
        for k in fv {
            mark[ix(f_target[k])] = 0;
        }
    }
    (0..n)
        .map(|v| {
            let d = to_u64(sym.degree(v));
            if d <= 1 {
                0.0
            } else {
                count[v] as f64 / (d * (d - 1)) as f64
            }
        })
        .collect()
}

/// Shortest-path lengths from `source` along `out`, over its weights (§ 2.3.6).
/// `None` if `out` has no weights.
#[must_use]
pub fn sssp(out: Lists<'_>, source: u32) -> Option<Vec<f64>> {
    let weights = out.weights?;
    let mut dist = vec![f64::INFINITY; out.n()];
    let mut done = vec![false; out.n()];
    dist[ix(source)] = 0.0;
    // Weights are never negative (§ 2.3.6), so the bit patterns order like the values.
    let mut heap = BinaryHeap::from([Reverse((0f64.to_bits(), source))]);
    while let Some(Reverse((_, u))) = heap.pop() {
        let u = ix(u);
        if std::mem::replace(&mut done[u], true) {
            continue;
        }
        for e in out.range(u) {
            let v = out.targets[e];
            let d = dist[u] + weights[e];
            if d < dist[ix(v)] {
                dist[ix(v)] = d;
                heap.push(Reverse((d.to_bits(), v)));
            }
        }
    }
    Some(dist)
}
