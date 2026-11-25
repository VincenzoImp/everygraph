// SPDX-License-Identifier: Apache-2.0 OR MIT

//! In-process reference kernels: sequential, textbook, and written for obviousness, not
//! speed. They let property-based tests compare an engine kernel against a reference
//! without leaving Rust; they are themselves checked against NetworkX, python-igraph and
//! scipy on every corpus run (`everygraph-oracle check --plugin reference`), so they are
//! a convenience, never the only witness.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use crate::graph::Graph;
use crate::kernel::{Kernel, Output};
use crate::projection::{Adjacency, Projection};
use crate::{exact_f64, ix, to_u64};

/// Computes `kernel` on the analytic graph. `None` when the oracle defines no answer for
/// this kernel on this graph (a directed-only kernel on an undirected graph, a partition
/// kernel without a partition).
#[must_use]
pub fn compute(g: &Graph, p: &Projection, kernel: &Kernel) -> Option<Output> {
    let n = p.len();
    let kept = |v: usize| p.kept_loops(v);
    Some(match kernel {
        Kernel::Adjacency => adjacency(p),
        Kernel::IngestReport => Output::Counts(p.report()),
        Kernel::Degree => Output::Ints(
            (0..n)
                // On a directed graph: distinct neighbours in either direction, the
                // degree in the undirected projection (design/DECISIONS.md, Q10).
                .map(|v| Some(to_u64(p.undirected.degree(v)) + 2 * kept(v)))
                .collect(),
        ),
        Kernel::DegreeOut | Kernel::DegreeIn if !p.directed => return None,
        Kernel::DegreeOut => Output::Ints(
            (0..n)
                .map(|v| Some(to_u64(p.out.degree(v)) + kept(v)))
                .collect(),
        ),
        Kernel::DegreeIn => Output::Ints(
            (0..n)
                .map(|v| Some(to_u64(p.inn.degree(v)) + kept(v)))
                .collect(),
        ),
        Kernel::PageRank { damping, weighted } => Output::Reals(
            pagerank(p, *damping, *weighted)
                .into_iter()
                .map(Some)
                .collect(),
        ),
        Kernel::Wcc => Output::Labels(wcc(&p.undirected)),
        Kernel::Scc if !p.directed => return None,
        Kernel::Scc => Output::Labels(scc(&p.out)),
        Kernel::Bfs { source } => Output::Ints(bfs(&p.out, ix(*source))?),
        Kernel::Sssp { source } => Output::Reals(sssp(&p.out, ix(*source))?),
        // Cohesion measures ignore kept loops (S2–S4): `p.undirected` has none.
        Kernel::CoreNumber => {
            Output::Ints(core_numbers(&p.undirected).into_iter().map(Some).collect())
        }
        Kernel::Triangles => Output::Ints(triangles(&p.undirected).into_iter().map(Some).collect()),
        Kernel::TriangleTotal => Output::Count(triangles(&p.undirected).iter().sum::<u64>() / 3),
        Kernel::Clustering => {
            Output::Reals(clustering(&p.undirected).into_iter().map(Some).collect())
        }
        Kernel::Transitivity => Output::Real(transitivity(&p.undirected)),
        Kernel::Modularity {
            resolution,
            weighted,
        } => Output::Real(modularity(p, g.partition.as_ref()?, *resolution, *weighted)),
        Kernel::Quotient => quotient(p, g.partition.as_ref()?)?,
    })
}

/// The out-adjacency with each kept loop listed once, in order, in its node's list.
fn adjacency(p: &Projection) -> Output {
    let mut offsets = vec![0u64];
    let mut targets = Vec::with_capacity(p.out.entries());
    for v in 0..p.len() {
        let list = p.out.list(v);
        let at = list.partition_point(|&u| ix(u) < v);
        targets.extend_from_slice(&list[..at]);
        if p.kept_loops(v) == 1 {
            targets.push(u32::try_from(v).expect("u32 ids"));
        }
        targets.extend_from_slice(&list[at..]);
        offsets.push(to_u64(targets.len()));
    }
    Output::Csr { offsets, targets }
}

/// Iterations that shrink the initial error by 1e-17 at damping `d` (at least 1).
fn iterations_for(damping: f64) -> u32 {
    if damping <= 0.0 {
        return 1;
    }
    let k = (1e-17f64).ln() / damping.ln();
    if k.is_finite() && k > 0.0 && k < 1e6 {
        // In range by the check above.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "0 < k < 1e6"
        )]
        let k = k.ceil() as u32;
        k.max(1)
    } else {
        1_000_000
    }
}

/// PageRank by power iteration to the fixed point (see [`Kernel::PageRank`]). A kept loop
/// is `A[v][v]`: `2w` undirected, `w` directed (S1).
#[must_use]
pub fn pagerank(p: &Projection, damping: f64, weighted: bool) -> Vec<f64> {
    let n = p.len();
    if n == 0 {
        return Vec::new();
    }
    let nf = f64::from(p.n);
    let w = |adj: &Adjacency, e: usize| if weighted { adj.weight_sum[e] } else { 1.0 };
    let diag = |v: usize| {
        let w = p.loop_pair_weight(v, weighted);
        if p.directed { w } else { 2.0 * w }
    };
    let out_w: Vec<f64> = (0..n)
        .map(|u| p.out.range(u).map(|e| w(&p.out, e)).sum::<f64>() + diag(u))
        .collect();
    let mut x = vec![1.0 / nf; n];
    for _ in 0..iterations_for(damping) {
        let dangling: f64 = (0..n).filter(|&u| out_w[u] == 0.0).map(|u| x[u]).sum();
        let base = (1.0 - damping) / nf + damping * dangling / nf;
        let next: Vec<f64> = (0..n)
            .map(|v| {
                let s: f64 = p
                    .inn
                    .range(v)
                    .map(|e| {
                        let u = ix(p.inn.targets[e]);
                        x[u] * w(&p.inn, e) / out_w[u]
                    })
                    .sum();
                let stay = if diag(v) > 0.0 {
                    x[v] * diag(v) / out_w[v]
                } else {
                    0.0
                };
                base + damping * (s + stay)
            })
            .collect();
        let diff: f64 = x.iter().zip(&next).map(|(a, b)| (a - b).abs()).sum();
        x = next;
        if diff == 0.0 {
            break;
        }
    }
    x
}

/// Components of a symmetric adjacency by breadth-first search, labelled by smallest id.
///
/// # Panics
///
/// If there are more than 2^32 nodes.
#[must_use]
pub fn wcc(sym: &Adjacency) -> Vec<u32> {
    let n = sym.n();
    let mut label = vec![u32::MAX; n];
    let mut queue = VecDeque::new();
    for s in 0..n {
        if label[s] != u32::MAX {
            continue;
        }
        let id = u32::try_from(s).expect("u32 ids");
        label[s] = id;
        queue.push_back(s);
        while let Some(u) = queue.pop_front() {
            for &v in sym.list(u) {
                if label[ix(v)] == u32::MAX {
                    label[ix(v)] = id;
                    queue.push_back(ix(v));
                }
            }
        }
    }
    label
}

/// Strongly connected components (iterative Tarjan), labelled by smallest id.
///
/// # Panics
///
/// If there are more than 2^32 nodes.
#[must_use]
pub fn scc(out: &Adjacency) -> Vec<u32> {
    const NONE: usize = usize::MAX;
    let n = out.n();
    let mut index = vec![NONE; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut label = vec![u32::MAX; n];
    let mut next = 0usize;
    for root in 0..n {
        if index[root] != NONE {
            continue;
        }
        // (node, position in its list)
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&(v, pos)) = call.last() {
            let list = out.list(v);
            if pos < list.len() {
                let w = ix(list[pos]);
                if let Some(top) = call.last_mut() {
                    top.1 += 1;
                }
                if index[w] == NONE {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut members = Vec::new();
                loop {
                    let w = stack.pop().expect("Tarjan stack");
                    on_stack[w] = false;
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                let id = members.iter().copied().min().expect("non-empty component");
                let id = u32::try_from(id).expect("u32 ids");
                for w in members {
                    label[w] = id;
                }
            }
        }
    }
    label
}

/// Hop distances from `source` along `out`. `None` if `source` is out of range.
///
/// # Panics
///
/// Never: queued nodes always have a distance.
#[must_use]
pub fn bfs(out: &Adjacency, source: usize) -> Option<Vec<Option<u64>>> {
    let n = out.n();
    if source >= n {
        return None;
    }
    let mut dist = vec![None; n];
    dist[source] = Some(0u64);
    let mut queue = VecDeque::from([source]);
    while let Some(u) = queue.pop_front() {
        let du = dist[u].expect("queued nodes have a distance");
        for &v in out.list(u) {
            if dist[ix(v)].is_none() {
                dist[ix(v)] = Some(du + 1);
                queue.push_back(ix(v));
            }
        }
    }
    Some(dist)
}

#[derive(PartialEq)]
struct Key(f64);
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Dijkstra from `source` with the smallest event weight of each pair.
#[must_use]
pub fn sssp(out: &Adjacency, source: usize) -> Option<Vec<Option<f64>>> {
    let n = out.n();
    if source >= n {
        return None;
    }
    let mut dist: Vec<Option<f64>> = vec![None; n];
    let mut done = vec![false; n];
    dist[source] = Some(0.0);
    let mut heap = BinaryHeap::from([Reverse((Key(0.0), source))]);
    while let Some(Reverse((Key(d), u))) = heap.pop() {
        if done[u] {
            continue;
        }
        done[u] = true;
        for e in out.range(u) {
            let v = ix(out.targets[e]);
            let nd = d + out.weight_min[e];
            if dist[v].is_none_or(|old| nd < old) {
                dist[v] = Some(nd);
                heap.push(Reverse((Key(nd), v)));
            }
        }
    }
    Some(dist)
}

/// Core numbers by repeatedly removing a node of smallest remaining degree.
#[must_use]
pub fn core_numbers(sym: &Adjacency) -> Vec<u64> {
    let n = sym.n();
    let mut deg: Vec<usize> = (0..n).map(|v| sym.degree(v)).collect();
    let mut alive = vec![true; n];
    let mut queue: BTreeSet<(usize, usize)> = (0..n).map(|v| (deg[v], v)).collect();
    let mut core = vec![0u64; n];
    let mut k = 0usize;
    while let Some((d, v)) = queue.pop_first() {
        k = k.max(d);
        core[v] = to_u64(k);
        alive[v] = false;
        for &u in sym.list(v) {
            let u = ix(u);
            if alive[u] {
                queue.remove(&(deg[u], u));
                deg[u] -= 1;
                queue.insert((deg[u], u));
            }
        }
    }
    core
}

/// Triangles through each node of a symmetric adjacency.
#[must_use]
pub fn triangles(sym: &Adjacency) -> Vec<u64> {
    let n = sym.n();
    let mut t = vec![0u64; n];
    for u in 0..n {
        for &v in sym.list(u) {
            let v = ix(v);
            if v <= u {
                continue;
            }
            // Common neighbours w > v: each triangle u < v < w is seen once.
            let (small, large) = if sym.degree(u) <= sym.degree(v) {
                (u, v)
            } else {
                (v, u)
            };
            for &w in sym.list(small) {
                if ix(w) > v && sym.has(large, w) {
                    t[u] += 1;
                    t[v] += 1;
                    t[ix(w)] += 1;
                }
            }
        }
    }
    t
}

/// Local clustering: `2 t / (d (d - 1))`, 0 where `d < 2`.
#[must_use]
pub fn clustering(sym: &Adjacency) -> Vec<f64> {
    let t = triangles(sym);
    (0..sym.n())
        .map(|v| {
            let d = to_u64(sym.degree(v));
            if d < 2 || t[v] == 0 {
                0.0
            } else {
                ratio(2 * t[v], d * (d - 1))
            }
        })
        .collect()
}

/// Transitivity: `3 T / triples`, 0 without triangles.
#[must_use]
pub fn transitivity(sym: &Adjacency) -> f64 {
    let t: u64 = triangles(sym).iter().sum();
    let triples: u64 = (0..sym.n())
        .map(|v| {
            let d = to_u64(sym.degree(v));
            d * d.saturating_sub(1)
        })
        .sum();
    if t == 0 { 0.0 } else { ratio(2 * t, triples) }
}

/// `a / b` as the correctly rounded double of the exact ratio when both fit in 53 bits.
fn ratio(a: u64, b: u64) -> f64 {
    let a = u32::try_from(a).map_or_else(|_| exact_f64(a), f64::from);
    let b = u32::try_from(b).map_or_else(|_| exact_f64(b), f64::from);
    a / b
}

/// Newman modularity of `partition` on the undirected projection, 0 without edges. A kept
/// loop of weight `w` is `A[v][v] = 2w`: `2w` in its node's strength and in its
/// community's `2L(c)` (S5).
#[must_use]
pub fn modularity(p: &Projection, partition: &[u32], resolution: f64, weighted: bool) -> f64 {
    let sym = &p.undirected;
    let mut inside: BTreeMap<u32, f64> = BTreeMap::new();
    let mut volume: BTreeMap<u32, f64> = BTreeMap::new();
    let mut total = 0.0; // 2W
    for u in 0..sym.n() {
        let c = partition[u];
        let diag = 2.0 * p.loop_pair_weight(u, weighted);
        let mut s = diag;
        let mut l = diag;
        for e in sym.range(u) {
            let w = if weighted { sym.weight_sum[e] } else { 1.0 };
            s += w;
            if partition[ix(sym.targets[e])] == c {
                l += w; // each pair twice
            }
        }
        *volume.entry(c).or_default() += s;
        *inside.entry(c).or_default() += l;
        total += s;
    }
    if total == 0.0 {
        return 0.0;
    }
    volume
        .iter()
        .map(|(c, &d)| inside[c] / total - resolution * (d / total).powi(2))
        .sum()
}

/// Quotient graph of a dense partition (labels below `n`), kept loops on the diagonal.
/// `None` otherwise.
#[must_use]
pub fn quotient(p: &Projection, partition: &[u32]) -> Option<Output> {
    let sym = &p.undirected;
    let n = sym.n();
    if partition.iter().any(|&c| ix(c) >= n.max(1)) {
        return None;
    }
    let k = partition.iter().map(|&c| ix(c) + 1).max().unwrap_or(0);
    let mut sizes = vec![0u64; k];
    for &c in partition {
        sizes[ix(c)] += 1;
    }
    let mut edges: BTreeMap<(u32, u32), u64> = BTreeMap::new();
    for u in 0..n {
        for &v in sym.list(u) {
            if ix(v) > u {
                let (a, b) = (partition[u], partition[ix(v)]);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        if p.kept_loops(u) == 1 {
            *edges.entry((partition[u], partition[u])).or_default() += 1;
        }
    }
    Some(Output::Quotient {
        sizes,
        edges: edges
            .into_iter()
            .map(|((a, b), w)| (a, b, exact_f64(w)))
            .collect(),
    })
}
