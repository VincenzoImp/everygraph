// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Single-source shortest paths over a per-entry cost column (#49): Δ*-stepping (Dong,
//! Gu, Sun and Zhang, SPAA'21) with bucket fusion (Zhang et al., `GraphIt`, CGO'20).
//!
//! The cost of an entry is the pair's smallest event cost (`docs/spec/graph-semantics.md`
//! § 4.2, indexed by entry offset as ADR 0028). The distance of a node is the smallest,
//! over the paths from the source, of the path's costs added from the source in `f64`.
//! `fl(d + c)` never decreases as `d` grows, so this is the one fixpoint of
//! `d(v) = min_u fl(d(u) + c(u, v))` that any label-correcting order reaches: the bits
//! do not depend on the threads, the source or the schedule. Only the speed does, and
//! `Δ` only sets the speed. A kept loop never shortens a path.
//!
//! # The algorithm
//!
//! Nodes whose distance dropped wait in an active set. A step takes the smallest
//! distance `d` among them, extracts every active node below `θ = (⌊d / Δ⌋ + 1)·Δ` and
//! relaxes their lists in parallel; a node whose distance drops becomes active again.
//! Bucket fusion: a worker that lowers a node below `θ` relaxes it at once, from a local
//! queue of at most [`FUSE`] nodes, instead of in a later step. That removes most steps
//! on graphs of large diameter.
//!
//! # Choosing Δ
//!
//! [`delta`] takes the median `M` of 1,024 costs at fixed entry positions and the mean
//! degree `d = m / n`: `Δ = M / d` when `d > 8`, else `16·M / d`. This is `AdaMW`'s rule
//! (D'Antonio et al., FCPC'25, § 4.1) with its thread count `T` replaced by 16, the
//! thread count of our reference machine, so that `Δ` depends on the data only.

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, PoisonError};

use everygraph_core::{Memory, MemoryKind, Reservation, par};

use crate::edge_map::{Error, Node, NodeList, NodeSet, PerNode, Plan, Row, Source};
use crate::weights;

/// Nodes a worker may hold in its local queue for bucket fusion (`GraphIt`'s 1,000).
/// First in, first out: a stack would make the order depth-first, whose repeated
/// relaxations grow without bound when one bucket holds most of the graph.
pub const FUSE: usize = 1024;

/// Costs sampled by [`delta`].
const SAMPLES: u64 = 1024;

const GRAIN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");

/// The parameters of [`sssp`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Sssp {
    /// The bucket width; `None`: [`delta`] of the graph. A width that is not positive
    /// and finite is taken as `None`.
    pub delta: Option<f64>,
}

/// The result of [`sssp`].
#[derive(Debug)]
pub struct Distances {
    /// The bits of each node's distance as an `f64`: `f64::INFINITY` where the node is
    /// not reached. [`Distances::get`] reads one.
    pub bits: Vec<AtomicU64>,
    /// The bucket width used.
    pub delta: f64,
    /// The budget held by `bits`.
    pub held: Reservation,
}

impl Distances {
    /// The distance of node `v`, if it is reached.
    ///
    /// # Panics
    ///
    /// If `v` is not a node.
    #[must_use]
    pub fn get(&self, v: u32) -> Option<f64> {
        let d = f64::from_bits(self.bits[usize::try_from(v).expect("an index")].load(Relaxed));
        d.is_finite().then_some(d)
    }
}

/// The bucket width chosen from the data alone (module documentation).
///
/// # Panics
///
/// If `costs` is shorter than the entries of `plan`'s source.
#[must_use]
pub fn delta<S: Source>(plan: &Plan<'_, '_, S>, costs: &[f64]) -> f64 {
    let m = plan.source().entries();
    let k = m.min(SAMPLES);
    let at = |i: u64| usize::try_from(u128::from(i) * u128::from(m) / u128::from(k));
    let mut sample: Vec<f64> = (0..k)
        .map(|i| costs[at(i).expect("an entry index")])
        .collect();
    sample.sort_unstable_by(f64::total_cmp);
    let median = match sample.get(sample.len() / 2) {
        Some(&x) if x > 0.0 => x,
        _ => sample.last().copied().unwrap_or(0.0),
    };
    if !(median > 0.0 && median.is_finite()) {
        return 1.0;
    }
    #[expect(clippy::cast_precision_loss, reason = "a mean degree")]
    let degree = m as f64 / f64::from(plan.nodes().max(1));
    let width = if degree > 8.0 {
        median / degree
    } else {
        16.0 * median / degree
    };
    if width > 0.0 && width.is_finite() {
        width
    } else {
        median
    }
}

/// Distances from `source` along the lists of `out`, with one cost per entry of its
/// source (module documentation).
///
/// # Errors
///
/// From the plan's source, or the state does not fit in the budget;
/// [`Error::Weight`] for a negative, NaN or infinite cost; [`Error::Argument`] if
/// `source` is not a node.
pub fn sssp<S: Source>(
    out: &Plan<'_, '_, S>,
    costs: &[f64],
    source: u32,
    p: &Sssp,
    memory: &Memory,
) -> Result<Distances, Error> {
    weights::check(costs, out.source().entries())?;
    let source = out
        .node(source)
        .ok_or(Error::Argument("sssp source: not a node"))?;
    let width = match p.delta {
        Some(w) if w > 0.0 && w.is_finite() => w,
        _ => delta(out, costs),
    };
    let n = out.nodes();
    let held = memory.reserve("sssp distances", MemoryKind::Anonymous, u64::from(n) * 8)?;
    let infinity = f64::INFINITY.to_bits();
    let bits: Vec<AtomicU64> = par::collect(ix(n), GRAIN, |_| AtomicU64::new(infinity));
    let dist = out.per_node(&bits)?;
    dist[source].store(0.0_f64.to_bits(), Relaxed);
    let active = out.node_set(memory)?;
    active.insert(source);
    let mut now = out.node_set(memory)?;
    let (mut waiting, mut frontier) = (NodeList::new(memory), NodeList::new(memory));
    let failed = Mutex::new(None);
    loop {
        active.list_into(&mut waiting)?;
        let list = waiting.as_slice()?;
        if list.is_empty() {
            break;
        }
        // Non-negative `f64`s order as their bits do.
        let lowest = par::map_ranges(list.len(), GRAIN, |r| {
            list[r].iter().map(|&v| dist[v].load(Relaxed)).min()
        });
        let lowest = f64::from_bits(lowest.into_iter().flatten().min().unwrap_or(infinity));
        let theta = ((lowest / width).floor() + 1.0) * width;
        let theta = if theta > lowest {
            theta
        } else {
            lowest.next_up()
        };
        par::for_each_range(list.len(), GRAIN, |r| {
            for &v in &list[r] {
                if f64::from_bits(dist[v].load(Relaxed)) < theta {
                    active.remove(v);
                    now.insert(v);
                }
            }
        });
        now.list_into(&mut frontier)?;
        now.clear();
        out.frontier_with(
            frontier.as_slice()?,
            VecDeque::new,
            |(): &mut (), row, w| {
                let step = Step {
                    dist,
                    costs,
                    theta,
                    active: &active,
                };
                step.relax(row, w.scratch);
                while let Some(v) = w.scratch.pop_front() {
                    match w.lists.get(v) {
                        Ok(row) => step.relax(row, w.scratch),
                        Err(e) => {
                            let mut failed = failed.lock().unwrap_or_else(PoisonError::into_inner);
                            failed.get_or_insert(e);
                            w.scratch.clear();
                        }
                    }
                }
            },
        )?;
        if let Some(e) = failed.lock().unwrap_or_else(PoisonError::into_inner).take() {
            return Err(e);
        }
    }
    Ok(Distances {
        bits,
        delta: width,
        held,
    })
}

/// What a relaxation reads and writes in one step of [`sssp`].
struct Step<'a, 'g> {
    dist: PerNode<'a, 'g, AtomicU64>,
    costs: &'a [f64],
    theta: f64,
    active: &'a NodeSet<'g>,
}

impl<'g> Step<'_, 'g> {
    /// Lowers the distances of the list of `row`'s node through it. A node lowered below
    /// `θ` goes on `queue` while it has room (bucket fusion), any other into the active
    /// set.
    fn relax(&self, row: Row<'_, 'g>, queue: &mut VecDeque<Node<'g>>) {
        let d = f64::from_bits(self.dist[row.node].load(Relaxed));
        for (&v, &c) in row.list.iter().zip(&self.costs[row.entries()]) {
            let new = (d + c).to_bits();
            let to = &self.dist[v];
            if new < to.load(Relaxed) && new < to.fetch_min(new, Relaxed) {
                if f64::from_bits(new) < self.theta && queue.len() < FUSE {
                    queue.push_back(v);
                } else {
                    self.active.insert(v);
                }
            }
        }
    }
}

fn ix(n: u32) -> usize {
    usize::try_from(n).expect("a node count fits in memory")
}
