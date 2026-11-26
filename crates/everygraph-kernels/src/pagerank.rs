// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `PageRank` and eigenvector centrality by pull power iteration (#47).
//!
//! # The equation
//!
//! [`pagerank`] iterates S1 of `docs/spec/graph-semantics.md` § 5.1:
//!
//! ```text
//! x(v) = (1 − α)·t(v) + α·( Σ_u x(u)·A[u][v] / s(u)  +  t(v)·Σ_{u : s(u) = 0} x(u) )
//! ```
//!
//! `α` is the damping, `s(u) = Σ_v A[u][v]` the out-strength ([`strength`]), and `t` the
//! teleport vector: `1/n` everywhere, or uniform over the seeds of
//! [`PageRank::personalize`]. Rank held by dangling nodes follows `t`. `A` is the matrix
//! of § 3: a pair's weight, or 1 without weights; a kept loop is the entry `v` in the list
//! of `v`, and counts `2w` on an undirected graph and `w` on a directed one. A pair of
//! weight 0 counts nothing, and a node whose out-weights are all 0 is dangling.
//!
//! Each iteration pulls along the [`Graph::lists`]: the in-lists of a directed graph,
//! the lists of an undirected one. It starts from `1/n` everywhere.
//!
//! # Stopping
//!
//! An iteration's change is the L1 norm `Σ_v |x'(v) − x(v)|`. The iteration stops after
//! the first iteration whose change is below [`PageRank::tol`], or after
//! [`PageRank::max_iter`] iterations. With `tol = 0` it runs exactly `max_iter`
//! iterations: the LDBC Graphalytics `PageRank`, whose formula is S1 with uniform teleport
//! (`docs/research/memory-bounded-algorithms.md`, addendum item 17); LDBC asks for f64.
//!
//! Other tools stop differently. `NetworkX` stops when the L1 change is below `n·tol`, so
//! with its default `tol = 1e-6` a graph of more than 2 million nodes stops after one
//! iteration; its backend reproduces that, this API does not. igraph's default (PRPACK)
//! solves the linear system and has no iteration count. Iterated to convergence, all
//! three reach the fixed point: the oracle compares them within 1e-9.
//!
//! # Determinism
//!
//! The ranks are the same bits for any thread count, chunk size, batch size and source
//! (DESIGN D14). Each node's sum over its list runs in one call, with four lanes fixed in
//! the code. The dangling mass, the change and the eigenvector's norm are exact sums
//! ([`ExactSum`]), so they do not depend on how nodes are grouped at all (PR #131 found
//! the lab's dangling mass following the chunk size). f32 state (`F = f32`,
//! `precision="low"`) keeps the contributions in f32 and the global sums exact.
//!
//! # Eigenvector centrality
//!
//! [`eigenvector`] runs the same pull, reductions and stopping rule on `y = x + A^T x`,
//! normalized to unit L2 norm after each iteration. The shift by `x` (as `NetworkX`'s
//! `eigenvector_centrality`) keeps a bipartite graph from oscillating and does not change
//! the eigenvector of the largest eigenvalue. On a graph where that eigenvalue is not
//! simple (two components with the same largest eigenvalue), the result is the limit
//! from the uniform start.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use everygraph_core::{ExactSum, Memory, MemoryKind, SegVec};

use crate::edge_map::{Error, NodeSet, PerNode, Plan, Row, Source, all};
use crate::weights;

/// The state precision of [`pagerank`]: `f64`, or `f32` for `precision="low"`.
pub trait Real: Copy + Send + Sync + PartialEq + 'static {
    /// Zero.
    const ZERO: Self;
    /// The nearest value to `x`.
    fn from_f64(x: f64) -> Self;
    /// The value as an `f64`, exactly.
    fn to_f64(self) -> f64;
    /// `self + other`.
    #[must_use]
    fn plus(self, other: Self) -> Self;
    /// `self * other`.
    #[must_use]
    fn times(self, other: Self) -> Self;
    /// `self / other`.
    #[must_use]
    fn over(self, other: Self) -> Self;
}

macro_rules! real {
    ($t:ty, $from:expr) => {
        impl Real for $t {
            const ZERO: Self = 0.0;
            #[inline(always)]
            fn from_f64(x: f64) -> Self {
                $from(x)
            }
            #[inline(always)]
            fn to_f64(self) -> f64 {
                self.into()
            }
            #[inline(always)]
            fn plus(self, other: Self) -> Self {
                self + other
            }
            #[inline(always)]
            fn times(self, other: Self) -> Self {
                self * other
            }
            #[inline(always)]
            fn over(self, other: Self) -> Self {
                self / other
            }
        }
    };
}

real!(f64, |x| x);
#[expect(
    clippy::cast_possible_truncation,
    reason = "rounding to f32 is the point"
)]
fn round_f32(x: f64) -> f32 {
    x as f32
}
real!(f32, round_f32);

/// A count as an `f64` (exact below 2^53).
#[expect(clippy::cast_precision_loss, reason = "counts below 2^53 are exact")]
fn float(x: u64) -> f64 {
    x as f64
}

/// The lists that a power iteration pulls along, with their weights.
#[derive(Clone, Copy, Debug)]
pub struct Graph<'p, 's, 'g, S: Source> {
    /// The in-lists of a directed graph; the lists of an undirected one.
    pub lists: &'p Plan<'s, 'g, S>,
    /// One weight per entry of `lists`' source, in its entry order; `None`: every pair
    /// counts 1.
    pub weights: Option<&'p [f64]>,
    /// Whether the graph is directed. On an undirected graph a kept loop (the entry `v`
    /// in the list of `v`) counts twice.
    pub directed: bool,
}

/// The parameters of [`pagerank`], with the defaults of `docs/api-sketch.md` § 4.
#[derive(Clone, Copy, Debug)]
pub struct PageRank<'a> {
    /// The probability of following an edge, `α`.
    pub damping: f64,
    /// Stop after the first iteration whose L1 change is below this; 0: never early.
    pub tol: f64,
    /// Iterations at most.
    pub max_iter: u32,
    /// The seed nodes of a personalized `PageRank` (repeats count once); `None`: uniform
    /// teleport.
    pub personalize: Option<&'a [u32]>,
}

impl Default for PageRank<'_> {
    fn default() -> Self {
        Self {
            damping: 0.85,
            tol: 1e-6,
            max_iter: 100,
            personalize: None,
        }
    }
}

/// The parameters of [`eigenvector`].
#[derive(Clone, Copy, Debug)]
pub struct Eigenvector {
    /// Stop after the first iteration whose L1 change is below this; 0: never early.
    pub tol: f64,
    /// Iterations at most.
    pub max_iter: u32,
}

impl Default for Eigenvector {
    fn default() -> Self {
        Self {
            tol: 1e-6,
            max_iter: 100,
        }
    }
}

/// The result of a power iteration: one value per node, in node order.
#[derive(Debug)]
pub struct Ranks<F: Copy> {
    /// The values.
    pub values: SegVec<F>,
    /// The iterations run.
    pub iterations: u32,
    /// The L1 change of the last iteration; infinite if none ran, or if `PageRank` ran
    /// with `tol = 0`, which does not compute it.
    pub change: f64,
}

/// The out-strength `s(u) = Σ_v A[u][v]` of every node, from its out-lists (the lists
/// of an undirected graph) and their weights: § 5.1's `s`, and the dangling test of
/// [`pagerank`].
///
/// # Errors
///
/// From the plan's source; [`Error::Weight`] for a negative, NaN or infinite weight;
/// [`Error::Memory`] if the strengths do not fit in the budget.
pub fn strength<S: Source>(
    out: &Plan<'_, '_, S>,
    weights: Option<&[f64]>,
    directed: bool,
    memory: &Memory,
) -> Result<SegVec<f64>, Error> {
    if let Some(w) = weights {
        weights::check(w, out.source().entries())?;
    }
    let mut s = SegVec::try_with_len(memory, "out-strength", ix(out.nodes()), 0.0)?;
    out.scan(one_segment(&mut s)?, all, |(): &mut (), s, i, row, _| {
        let v = row.node;
        s[i] = match weights {
            Some(w) => row
                .list
                .iter()
                .zip(&w[row.entries()])
                .fold(0.0, |sum, (&u, &w)| {
                    sum + if !directed && u == v { 2.0 * w } else { w }
                }),
            None => row.list.iter().fold(0.0, |sum, &u| {
                sum + if !directed && u == v { 2.0 } else { 1.0 }
            }),
        };
    })?;
    Ok(s)
}

/// `PageRank` by pull power iteration (module documentation): `strength` is
/// [`strength`] of the graph's out-lists.
///
/// # Errors
///
/// From the plan's source; [`Error::Weight`] for a bad weight; [`Error::Argument`] for
/// a damping outside `[0, 1]`, an empty seed set or a seed that is not a node;
/// [`Error::Invalid`] if `strength` has fewer values than the nodes; [`Error::Memory`]
/// if the state does not fit in the budget.
pub fn pagerank<S: Source, F: Real>(
    g: &Graph<'_, '_, '_, S>,
    strength: &[f64],
    p: &PageRank<'_>,
    memory: &Memory,
) -> Result<Ranks<F>, Error> {
    let plan = g.lists;
    let n = ix(plan.nodes());
    if !(0.0..=1.0).contains(&p.damping) {
        return Err(Error::Argument("damping: not between 0 and 1"));
    }
    if let Some(w) = g.weights {
        weights::check(w, plan.source().entries())?;
    }
    let s = plan.per_node(strength)?;
    let seeds = match p.personalize {
        Some(ids) => Some(seed_set(plan, ids, memory)?),
        None => None,
    };
    let looped = loops(g, memory)?;
    let inv_n = 1.0 / float(n as u64);
    let mut contrib = SegVec::try_with_len(memory, "pagerank contributions", n, F::ZERO)?;
    let mut next = SegVec::try_with_len(memory, "pagerank next", n, F::ZERO)?;
    let mut rank = SegVec::try_with_len(memory, "pagerank", n, F::from_f64(inv_n))?;
    let sinks: u64 = plan.vertices(one_segment(&mut contrib)?, |sinks, c, i, v, _| {
        if s[v] == 0.0 {
            *sinks += 1;
        } else {
            c[i] = F::from_f64(inv_n / s[v]);
        }
    })?;
    let mut dangling = float(sinks) * inv_n;
    let damping = F::from_f64(p.damping);
    // With `tol = 0` no change is needed, and the ranks only after the last iteration.
    let track = p.tol > 0.0;
    let (mut iterations, mut change) = (0, f64::INFINITY);
    while iterations < p.max_iter && change >= p.tol {
        let keep = track || iterations + 1 == p.max_iter;
        // `t(v)·((1 − α) + α·D)`, for the nodes where `t(v)` is not 0.
        let coef = (1.0 - p.damping) + p.damping * dangling;
        let base = F::from_f64(match &seeds {
            Some((_, count)) => coef / float(*count),
            None => coef * inv_n,
        });
        let c = plan.per_node(contrib.as_slice().ok_or(SEGMENTS)?)?;
        let out = (one_segment(&mut next)?, one_segment(&mut rank)?);
        let sums: (ExactSum, ExactSum) = plan.scan(
            out,
            all,
            |acc: &mut (ExactSum, ExactSum), (next, rank), i, row, _| {
                let v = row.node;
                let teleport = match &seeds {
                    Some((set, _)) if !set.contains(v) => F::ZERO,
                    _ => base,
                };
                let r = teleport.plus(damping.times(pull(c, &row, g.weights, looped.as_ref())));
                if track {
                    ExactSum::add(&mut acc.1, (r.to_f64() - rank[i].to_f64()).abs());
                }
                if keep {
                    rank[i] = r;
                }
                if s[v] == 0.0 {
                    ExactSum::add(&mut acc.0, r.to_f64());
                    next[i] = F::ZERO;
                } else {
                    next[i] = r.over(F::from_f64(s[v]));
                }
            },
        )?;
        dangling = sums.0.value();
        if track {
            change = sums.1.value();
        }
        std::mem::swap(&mut contrib, &mut next);
        iterations += 1;
    }
    Ok(Ranks {
        values: rank,
        iterations,
        change,
    })
}

/// Eigenvector centrality by the power iteration of the module documentation: the
/// eigenvector of the largest eigenvalue of `A^T` (in-lists), with unit L2 norm and no
/// negative value.
///
/// # Errors
///
/// From the plan's source; [`Error::Weight`] for a bad weight; [`Error::Memory`] if the
/// state does not fit in the budget.
pub fn eigenvector<S: Source>(
    g: &Graph<'_, '_, '_, S>,
    p: &Eigenvector,
    memory: &Memory,
) -> Result<Ranks<f64>, Error> {
    let plan = g.lists;
    let n = ix(plan.nodes());
    if let Some(w) = g.weights {
        weights::check(w, plan.source().entries())?;
    }
    let looped = loops(g, memory)?;
    let start = 1.0 / float(n as u64);
    let mut x = SegVec::try_with_len(memory, "eigenvector", n, start)?;
    let mut y = SegVec::try_with_len(memory, "eigenvector next", n, 0.0)?;
    let (mut iterations, mut change) = (0, f64::INFINITY);
    while iterations < p.max_iter && change >= p.tol {
        let xs = plan.per_node(x.as_slice().ok_or(SEGMENTS)?)?;
        let squares: ExactSum = plan.scan(
            one_segment(&mut y)?,
            all,
            |sq: &mut ExactSum, y, i, row, _| {
                let v = row.node;
                y[i] = xs[v] + pull(xs, &row, g.weights, looped.as_ref());
                sq.add(y[i] * y[i]);
            },
        )?;
        let norm = squares.value().sqrt();
        let ys = plan.per_node(y.as_slice().ok_or(SEGMENTS)?)?;
        let moved: ExactSum =
            plan.vertices(one_segment(&mut x)?, |d: &mut ExactSum, x, i, v, _| {
                let new = ys[v] / norm;
                ExactSum::add(d, (new - x[i]).abs());
                x[i] = new;
            })?;
        change = moved.value();
        iterations += 1;
    }
    Ok(Ranks {
        values: x,
        iterations,
        change,
    })
}

/// The `k` largest values with their nodes, largest first; equal values in node order
/// (`docs/api-sketch.md` § 2.12). Values are compared by `f64::total_cmp`.
///
/// # Errors
///
/// [`Error::Memory`] if `min(k, n)` results do not fit in the budget.
pub fn top<F: Real>(values: &[F], k: usize, memory: &Memory) -> Result<SegVec<(u32, F)>, Error> {
    let k = k.min(values.len());
    let bytes = (k as u64 + 1) * std::mem::size_of::<(u32, F)>() as u64;
    let _heap = memory.reserve("top-k heap", MemoryKind::Anonymous, bytes)?;
    // A min-heap of the best `k` so far: its top is the worst of them.
    let mut heap = BinaryHeap::new();
    for (id, &x) in (0_u32..).zip(values) {
        heap.push(Reverse(Key(x.to_f64(), Reverse(id))));
        if heap.len() > k {
            heap.pop();
        }
    }
    let mut out = SegVec::try_with_len(memory, "top-k", k, (0, F::ZERO))?;
    let mut at = k;
    while let Some(Reverse(Key(_, Reverse(id)))) = heap.pop() {
        at -= 1;
        if let Some(slot) = out.get_mut(at) {
            *slot = (id, values[ix(id)]);
        }
    }
    Ok(out)
}

/// A value and a node, ordered by value, then by node.
#[derive(Clone, Copy, Debug)]
struct Key(f64, Reverse<u32>);

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0).then(self.1.cmp(&other.1))
    }
}

const SEGMENTS: Error = Error::Invalid("node state longer than one segment (#m3-15)");

fn one_segment<T: Copy>(v: &mut SegVec<T>) -> Result<&mut [T], Error> {
    v.as_mut_slice().ok_or(SEGMENTS)
}

fn ix(n: u32) -> usize {
    usize::try_from(n).expect("a node count fits in memory")
}

/// The seed set of a personalized `PageRank`, and the number of distinct seeds.
fn seed_set<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    ids: &[u32],
    memory: &Memory,
) -> Result<(NodeSet<'g>, u64), Error> {
    let set = plan.node_set(memory)?;
    let mut count = 0;
    for &id in ids {
        let v = plan
            .node(id)
            .ok_or(Error::Argument("personalize: a seed that is not a node"))?;
        count += u64::from(set.insert(v));
    }
    if count == 0 {
        return Err(Error::Argument("personalize: no seed"));
    }
    Ok((set, count))
}

/// The nodes with a kept loop on an undirected graph, which count it twice; `None` if
/// there are none, or the graph is directed.
fn loops<'g, S: Source>(
    g: &Graph<'_, '_, 'g, S>,
    memory: &Memory,
) -> Result<Option<NodeSet<'g>>, Error> {
    if g.directed {
        return Ok(None);
    }
    let set = g.lists.node_set(memory)?;
    let found: u64 = g.lists.scan((), all, |found: &mut u64, (), _, row, _| {
        if row.list.contains(&row.node) {
            set.insert(row.node);
            *found += 1;
        }
    })?;
    Ok((found > 0).then_some(set))
}

/// `Σ_u x(u)·A[u][v]` over the list of `v` in `row`: four lanes, then the second count
/// of a kept loop on an undirected graph.
#[inline(always)]
fn pull<'g, F: Real>(
    x: PerNode<'_, 'g, F>,
    row: &Row<'_, 'g>,
    weights: Option<&[f64]>,
    looped: Option<&NodeSet<'g>>,
) -> F {
    let list = row.list;
    let (mut s0, mut s1, mut s2, mut s3) = (F::ZERO, F::ZERO, F::ZERO, F::ZERO);
    match weights {
        None => {
            let mut quads = list.chunks_exact(4);
            for q in &mut quads {
                s0 = s0.plus(x[q[0]]);
                s1 = s1.plus(x[q[1]]);
                s2 = s2.plus(x[q[2]]);
                s3 = s3.plus(x[q[3]]);
            }
            for &u in quads.remainder() {
                s0 = s0.plus(x[u]);
            }
        }
        Some(w) => {
            let w = &w[row.entries()];
            let term = |e: usize| x[list[e]].times(F::from_f64(w[e]));
            let quads = list.len() / 4 * 4;
            for e in (0..quads).step_by(4) {
                s0 = s0.plus(term(e));
                s1 = s1.plus(term(e + 1));
                s2 = s2.plus(term(e + 2));
                s3 = s3.plus(term(e + 3));
            }
            for e in quads..list.len() {
                s0 = s0.plus(term(e));
            }
        }
    }
    let sum = s0.plus(s1).plus(s2.plus(s3));
    match looped {
        Some(set) if set.contains(row.node) => sum.plus(loop_pull(x, row, weights)),
        _ => sum,
    }
}

/// `x(v)·w` for the loop entries of `v`, the second half of its `2w`.
#[cold]
fn loop_pull<'g, F: Real>(x: PerNode<'_, 'g, F>, row: &Row<'_, 'g>, weights: Option<&[f64]>) -> F {
    let v = row.node;
    let w = match weights {
        Some(w) => row
            .list
            .iter()
            .zip(&w[row.entries()])
            .filter(|&(&u, _)| u == v)
            .fold(0.0, |s, (_, &w)| s + w),
        None => float(row.list.iter().filter(|&&u| u == v).count() as u64),
    };
    x[v].times(F::from_f64(w))
}
