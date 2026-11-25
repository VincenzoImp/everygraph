// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The framework's proof kernels: `PageRank`, connected components and BFS, each written
//! once against [`Plan`] and run unchanged over every [`Source`] (ADR 0012, #44).
//!
//! They are the lab's kernels (`lab/b1-core/crates/glab-core/src/kernels.rs`) with the
//! same algorithms, constants and work partition, and nothing more: undirected graphs
//! without kept loops for `PageRank`, unweighted, a fixed number of iterations. The
//! production kernels, with the semantics of `docs/spec/graph-semantics.md`, are #47
//! (`PageRank`), #48 (components) and #49 (traversals), which replace these.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use everygraph_core::{Memory, MemoryKind, Reservation, SegVec, par};

use crate::edge_map::{Error, Node, NodeCells, NodeList, PerNode, Plan, Source, all, ix};

/// Nodes per task of the per-node passes outside the drivers (the lab's 4,096).
const GRAIN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");

/// The state precision of [`pagerank`].
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

/// A degree, an index or a count as an `f64` (exact below 2^53).
#[expect(clippy::cast_precision_loss, reason = "counts below 2^53 are exact")]
fn float(x: u64) -> f64 {
    x as f64
}

/// The sum of `contrib` over `list`, with four accumulators so that the adds do not
/// serialize the loads (the lab's `gather_sum`).
#[inline(always)]
fn gather<'g, F: Real>(contrib: PerNode<'_, 'g, F>, list: &[Node<'g>]) -> F {
    let (mut s0, mut s1, mut s2, mut s3) = (F::ZERO, F::ZERO, F::ZERO, F::ZERO);
    let mut quads = list.chunks_exact(4);
    for q in &mut quads {
        s0 = s0.plus(contrib[q[0]]);
        s1 = s1.plus(contrib[q[1]]);
        s2 = s2.plus(contrib[q[2]]);
        s3 = s3.plus(contrib[q[3]]);
    }
    for &u in quads.remainder() {
        s0 = s0.plus(contrib[u]);
    }
    s0.plus(s1).plus(s2.plus(s3))
}

fn one_segment<T: Copy>(v: &mut SegVec<T>) -> Result<&mut [T], Error> {
    v.as_mut_slice().ok_or(Error::Invalid(
        "node state longer than one segment (#m3-15)",
    ))
}

/// Pull `PageRank` of an undirected graph: `iterations` power iterations from the
/// uniform vector, uniform teleport, dangling mass spread uniformly (the lab's kernel).
///
/// # Errors
///
/// From the plan's source, or the state does not fit in the budget.
pub fn pagerank<S: Source, F: Real>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    damping: f64,
    iterations: u32,
) -> Result<SegVec<F>, Error> {
    let n = ix(plan.nodes().into());
    let inv_n = 1.0 / float(n as u64);
    let mut contrib = SegVec::try_with_len(memory, "pagerank contributions", n, F::ZERO)?;
    let mut next = SegVec::try_with_len(memory, "pagerank next", n, F::ZERO)?;
    let mut rank = SegVec::try_with_len(memory, "pagerank", n, F::from_f64(inv_n))?;
    let mut dangling: f64 = plan.vertices(one_segment(&mut contrib)?, |dang, c, i, _, d| {
        if d == 0 {
            *dang += inv_n;
        } else {
            c[i] = F::from_f64(inv_n / f64::from(d));
        }
    })?;
    for it in 0..iterations {
        let base = F::from_f64((1.0 - damping) * inv_n + damping * dangling * inv_n);
        let dmp = F::from_f64(damping);
        let last = it + 1 == iterations;
        let c = contrib.as_slice().ok_or(Error::Invalid(
            "node state longer than one segment (#m3-15)",
        ))?;
        let c = plan.per_node(c)?;
        let out = (one_segment(&mut next)?, one_segment(&mut rank)?);
        dangling = plan.scan(out, all, |dang: &mut f64, (next, rank), i, _, list| {
            let r = base.plus(dmp.times(gather(c, list)));
            if list.is_empty() {
                *dang += r.to_f64();
                next[i] = F::ZERO;
            } else {
                next[i] = r.over(F::from_f64(float(list.len() as u64)));
            }
            if last {
                rank[i] = r;
            }
        })?;
        std::mem::swap(&mut contrib, &mut next);
    }
    Ok(rank)
}

/// Hooks the larger of the two roots under the smaller: the final label of a component
/// is its smallest node, whatever order the links land in (the lab's `uf_link`).
#[inline]
fn link<'g>(comp: &NodeCells<'g>, u: Node<'g>, v: Node<'g>) {
    let (mut p1, mut p2) = (comp.load(u, Relaxed), comp.load(v, Relaxed));
    while p1 != p2 {
        let (high, low) = if p1 > p2 { (p1, p2) } else { (p2, p1) };
        let ph = comp.load(high, Relaxed);
        if ph == low || (ph == high && comp.compare_exchange(high, high, low, Relaxed).is_ok()) {
            break;
        }
        p1 = comp.load(comp.load(high, Relaxed), Relaxed);
        p2 = comp.load(low, Relaxed);
    }
}

/// The most frequent label among 1,024 fixed samples (ties: the smallest label).
fn sample_frequent<'g, S: Source>(
    plan: &Plan<'_, 'g, S>,
    comp: &NodeCells<'g>,
) -> Option<Node<'g>> {
    let n = u64::from(plan.nodes());
    let mut seen: Vec<(Node<'g>, u32)> = Vec::new();
    let mut x = 0x9E37_79B9_7F4A_7C15_u64;
    for _ in 0..1024 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let label = comp.load(plan.node(u32::try_from(x.checked_rem(n)?).ok()?)?, Relaxed);
        match seen.iter_mut().find(|(l, _)| *l == label) {
            Some((_, c)) => *c += 1,
            None => seen.push((label, 1)),
        }
    }
    seen.into_iter()
        .max_by_key(|&(l, c)| (c, std::cmp::Reverse(l)))
        .map(|(l, _)| l)
}

/// Weakly connected components of a symmetric graph by Afforest (two sampling rounds,
/// then every list outside the most frequent component; the lab's `wcc_afforest`).
/// Each label is the smallest node of its component; the labels are returned as the
/// atomics they were computed in, with the reservation that holds them.
///
/// # Errors
///
/// From the plan's source, or the labels do not fit in the budget.
pub fn wcc<S: Source>(plan: &Plan<'_, '_, S>, memory: &Memory) -> Result<Atomics, Error> {
    const ROUNDS: usize = 2;
    let comp = plan.node_cells(memory)?;
    for r in 0..ROUNDS {
        plan.scan((), all, |(): &mut (), (), _, u, list| {
            if let Some(&v) = list.get(r) {
                link(&comp, u, v);
            }
        })?;
        comp.flatten();
    }
    if let Some(giant) = sample_frequent(plan, &comp) {
        let in_giant = |u| comp.load(u, Relaxed) == giant;
        plan.scan(
            (),
            |u| !in_giant(u),
            |(): &mut (), (), _, u, list| {
                for &v in list.get(ROUNDS..).unwrap_or_default() {
                    link(&comp, u, v);
                }
            },
        )?;
        comp.flatten();
    }
    Ok(comp.into_ids())
}

/// Per-node values in atomics, and the reservation that holds them.
pub type Atomics = (Vec<AtomicU32>, Reservation);

/// Not reached.
pub const UNREACHED: u32 = u32::MAX;

/// Hop distances from `source` by direction-optimizing BFS (Beamer; the lab's `bfs_do`
/// with α = 15, β = 18). Bottom-up steps read the lists of the unreached nodes, so they
/// need a symmetric graph; on a directed one pass `symmetric = false` and every step is
/// top-down. Returned as the atomics they were computed in, with their reservation.
///
/// # Errors
///
/// From the plan's source, or the state does not fit in the budget; [`Error::Invalid`]
/// if `source` is not a node.
pub fn bfs<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    source: u32,
    symmetric: bool,
) -> Result<Atomics, Error> {
    const ALPHA: u64 = 15;
    const BETA: u64 = 18;
    let n = plan.nodes();
    let source = plan
        .node(source)
        .ok_or(Error::Invalid("bfs source: not a node"))?;
    let reservation = memory.reserve("bfs depths", MemoryKind::Anonymous, u64::from(n) * 4)?;
    let depth: Vec<AtomicU32> = par::collect(ix(n.into()), GRAIN, |_| AtomicU32::new(UNREACHED));
    let at = plan.per_node(&depth)?;
    at[source].store(0, Relaxed);
    let mut frontier = NodeList::new(memory);
    frontier.push(source)?;
    let mut next = plan.node_set(memory)?;
    let mut level = 0;
    let mut edges_to_check = plan.source().entries();
    let mut scout = u64::from(plan.degree(source));
    while !frontier.is_empty() {
        if symmetric && scout > edges_to_check / ALPHA {
            let mut front = plan.node_set(memory)?;
            let list = frontier.as_slice()?;
            par::for_each_range(list.len(), GRAIN, |r| {
                for &u in &list[r] {
                    front.insert(u);
                }
            });
            let mut awake = frontier.len() as u64;
            loop {
                let old = awake;
                awake = plan.scan(
                    (),
                    |v| at[v].load(Relaxed) == UNREACHED,
                    |found: &mut u64, (), _, v, list| {
                        if list.iter().any(|&u| front.contains(u)) {
                            at[v].store(level + 1, Relaxed);
                            next.insert(v);
                            *found += 1;
                        }
                    },
                )?;
                level += 1;
                std::mem::swap(&mut front, &mut next);
                next.clear();
                if awake == 0 || (awake < old && awake <= u64::from(n) / BETA) {
                    break;
                }
            }
            front.list_into(&mut frontier)?;
            scout = 1;
        } else {
            edges_to_check = edges_to_check.saturating_sub(scout);
            scout = plan.frontier(frontier.as_slice()?, |scout: &mut u64, _, list| {
                for &v in list {
                    let d = &at[v];
                    if d.load(Relaxed) == UNREACHED
                        && d.compare_exchange(UNREACHED, level + 1, Relaxed, Relaxed)
                            .is_ok()
                    {
                        next.insert(v);
                        *scout += u64::from(plan.degree(v));
                    }
                }
            })?;
            next.list_into(&mut frontier)?;
            next.clear();
            level += 1;
        }
    }
    Ok((depth, reservation))
}
