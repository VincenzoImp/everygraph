// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Breadth-first traversals (#49): direction-optimizing BFS from one source, and
//! bit-parallel BFS from batches of sources. Weighted distances are in
//! [`crate::sssp`].
//!
//! Both follow the out-lists of a directed graph, or the lists of an undirected one.
//! A kept loop is never a step (`docs/spec/graph-semantics.md` § 5). Distances are the
//! unique hop counts, so they do not depend on the order in which racing updates land;
//! parents are chosen by a rule on the distances alone (§ 2.12 of `docs/api-sketch.md`).

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

use everygraph_core::{Memory, MemoryKind, Reservation, SegVec, par, usize_of};

use crate::edge_map::{Error, NodeList, Plan, Reduce, Source, one_segment, one_segment_mut};

/// The distance and parent of a node that is not reached.
pub const UNREACHED: u32 = u32::MAX;

/// Nodes per task of the per-node passes outside the drivers.
const GRAIN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");

/// Beamer's switch to bottom-up: when the frontier's out-edges exceed the unexplored
/// edges over `ALPHA` (the lab's 15, FINDINGS § B1).
const ALPHA: u64 = 15;
/// Back to top-down when a shrinking frontier holds at most `n / BETA` nodes (the lab's
/// 18).
const BETA: u64 = 18;

/// The parameters of [`bfs`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Bfs {
    /// Stop at this distance; `None`: go as far as the graph goes.
    pub max_hops: Option<u32>,
    /// Compute the parent of every reached node.
    pub parents: bool,
}

/// The result of [`bfs`].
#[derive(Debug)]
pub struct Hops {
    /// Hops from the source; [`UNREACHED`] where it is not reached.
    pub distance: Vec<AtomicU32>,
    /// With [`Bfs::parents`]: the smallest node one hop closer to the source with an
    /// edge to the node; [`UNREACHED`] at the source and where it is not reached.
    pub parent: Option<Vec<AtomicU32>>,
    /// The number of nodes at each distance, from 0.
    pub levels: Vec<u64>,
    /// The budget held by `distance` and `parent`.
    pub held: Reservation,
}

/// Hop distances from `source` by direction-optimizing BFS (Beamer, SC'12), along the
/// lists of `out`.
///
/// A top-down step pushes from the frontier along `out`. A bottom-up step scans the
/// nodes not yet reached and looks for a frontier node in their lists of `inn`: the
/// in-lists of a directed graph, or `out` itself on an undirected one. Without `inn`
/// every step is top-down. The two plans share their brand ([`Plan::with_pair`]).
///
/// # Errors
///
/// From the plans' sources, or the state does not fit in the budget;
/// [`Error::Argument`] if `source` is not a node; [`Error::Invalid`] if `inn` has
/// another number of entries than `out`, so is not the in-lists of the same graph.
pub fn bfs<'g, S: Source>(
    out: &Plan<'_, 'g, S>,
    inn: Option<&Plan<'_, 'g, S>>,
    source: u32,
    p: &Bfs,
    memory: &Memory,
) -> Result<Hops, Error> {
    let n = out.nodes();
    let source = out
        .node(source)
        .ok_or(Error::Argument("bfs source: not a node"))?;
    if inn.is_some_and(|inn| inn.source().entries() != out.source().entries()) {
        return Err(Error::Invalid(
            "bfs: in-lists with another number of entries",
        ));
    }
    let words = if p.parents { 8 } else { 4 };
    let held = memory.reserve("bfs state", MemoryKind::Anonymous, u64::from(n) * words)?;
    let depth = unreached(n);
    let at = out.per_node(&depth)?;
    at[source].store(0, Relaxed);
    let max_hops = p.max_hops.unwrap_or(u32::MAX - 1);
    let mut levels = vec![1];
    let mut frontier = NodeList::new(memory);
    frontier.push(source)?;
    let mut next = out.node_set(memory)?;
    let mut level = 0;
    let mut edges_to_check = out.source().entries();
    let mut scout = u64::from(out.degree(source));
    while !frontier.is_empty() && level < max_hops {
        match inn {
            Some(inn) if scout > edges_to_check / ALPHA => {
                let mut front = out.node_set(memory)?;
                let list = frontier.as_slice()?;
                par::for_each_range(list.len(), GRAIN, |r| {
                    for &u in &list[r] {
                        front.insert(u);
                    }
                });
                let mut awake = frontier.len() as u64;
                loop {
                    let old = awake;
                    awake = inn.scan(
                        (),
                        |v| at[v].load(Relaxed) == UNREACHED,
                        |found: &mut u64, (), _, row, _| {
                            if row.list.iter().any(|&u| front.contains(u)) {
                                at[row.node].store(level + 1, Relaxed);
                                next.insert(row.node);
                                *found += 1;
                            }
                        },
                    )?;
                    level += 1;
                    std::mem::swap(&mut front, &mut next);
                    next.clear();
                    if awake > 0 {
                        levels.push(awake);
                    }
                    let shrunk = awake < old && awake <= u64::from(n) / BETA;
                    if awake == 0 || shrunk || level >= max_hops {
                        break;
                    }
                }
                front.list_into(&mut frontier)?;
                // As GAP's reference BFS: the next step is top-down, and the edges that
                // bottom-up steps examined are not counted.
                scout = 1;
            }
            _ => {
                edges_to_check = edges_to_check.saturating_sub(scout);
                scout = out.frontier(frontier.as_slice()?, |scout: &mut u64, row, _| {
                    for &v in row.list {
                        let d = &at[v];
                        if d.load(Relaxed) == UNREACHED
                            && d.compare_exchange(UNREACHED, level + 1, Relaxed, Relaxed)
                                .is_ok()
                        {
                            next.insert(v);
                            *scout += u64::from(out.degree(v));
                        }
                    }
                })?;
                next.list_into(&mut frontier)?;
                next.clear();
                level += 1;
                if !frontier.is_empty() {
                    levels.push(frontier.len() as u64);
                }
            }
        }
    }
    let parent = if p.parents {
        let parent = unreached(n);
        let up = out.per_node(&parent)?;
        out.scan(
            (),
            |u| at[u].load(Relaxed) < max_hops,
            |(): &mut (), (), _, row, _| {
                let d = at[row.node].load(Relaxed) + 1;
                for &v in row.list {
                    if at[v].load(Relaxed) == d {
                        up[v].fetch_min(row.node.get(), Relaxed);
                    }
                }
            },
        )?;
        Some(parent)
    } else {
        None
    };
    Ok(Hops {
        distance: depth,
        parent,
        levels,
        held,
    })
}

/// `n` cells holding [`UNREACHED`], filled in parallel; the caller reserves them.
fn unreached(n: u32) -> Vec<AtomicU32> {
    par::collect(usize_of(n), GRAIN, |_| AtomicU32::new(UNREACHED))
}

/// Sources per batch of [`multi_bfs`]: the bits of a word.
pub const BATCH: usize = 64;

/// The result of [`multi_bfs`].
#[derive(Debug)]
pub struct MultiHops {
    /// `levels[d][k]`: the number of nodes at distance `d` from source `k`, for `d` up to
    /// the largest distance from any source.
    pub levels: Vec<Vec<u64>>,
    /// Per node: the sum of its distances from the sources that reach it.
    pub distance_sum: SegVec<u64>,
    /// Per node: the number of sources that reach it.
    pub reached: SegVec<u32>,
}

/// One count per source of a batch, added lane by lane.
#[derive(Clone, Copy)]
struct PerSource([u64; BATCH]);

impl Reduce for PerSource {
    const ZERO: Self = Self([0; BATCH]);
    fn add(mut self, other: Self) -> Self {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a += b;
        }
        self
    }
}

/// Hop distances from many sources at once, by bit-parallel BFS (Then et al., VLDB
/// 2014): batches of [`BATCH`] sources, one bit per source in a word per node. Each step
/// pulls along `inn` (the in-lists of a directed graph, the lists of an undirected one):
/// a node takes the frontier bits of its in-neighbours that it has not seen. Every node
/// writes only its own words, and the counts are integers, so the result does not depend
/// on the schedule.
///
/// For sampled centralities and diameter estimates: per source, the nodes at each
/// distance; per node, its summed distance and the sources that reach it. A step scans
/// every node that some source of the batch has not reached, so on a graph of large
/// diameter the cost is about `diameter × n` per batch.
///
/// # Errors
///
/// From the plan's source, or the state does not fit in the budget;
/// [`Error::Argument`] if a source is not a node.
pub fn multi_bfs<S: Source>(
    inn: &Plan<'_, '_, S>,
    sources: &[u32],
    max_hops: Option<u32>,
    memory: &Memory,
) -> Result<MultiHops, Error> {
    let n = usize_of(inn.nodes());
    let max_hops = max_hops.unwrap_or(u32::MAX);
    let mut distance_sum = SegVec::try_with_len(memory, "distance sums", n, 0)?;
    let mut reached = SegVec::try_with_len(memory, "sources reached", n, 0)?;
    let mut levels: Vec<Vec<u64>> = Vec::new();
    let _held = memory.reserve("seen bits", MemoryKind::Anonymous, n as u64 * 8)?;
    let mut frontier = SegVec::try_with_len(memory, "frontier bits", n, 0_u64)?;
    let mut next = SegVec::try_with_len(memory, "next bits", n, 0_u64)?;
    for (b, batch) in sources.chunks(BATCH).enumerate() {
        let first = b * BATCH;
        let seen: Vec<AtomicU64> = par::collect(n, GRAIN, |_| AtomicU64::new(0));
        let at = inn.per_node(&seen)?;
        frontier.fill(0);
        next.fill(0);
        let full = u64::MAX >> (BATCH - batch.len());
        for (k, &s) in batch.iter().enumerate() {
            let v = inn.node(s).ok_or(Error::Argument(
                "multi-source bfs: a source that is not a node",
            ))?;
            at[v].fetch_or(1 << k, Relaxed);
            if let Some(word) = frontier.get_mut(v.index()) {
                *word |= 1 << k;
            }
        }
        add_level(&mut levels, 0, first, &[1; BATCH][..batch.len()]);
        let mut level = 0;
        while level < max_hops {
            level += 1;
            let f = inn.per_node(one_segment(&frontier)?)?;
            let out = (
                one_segment_mut(&mut next)?,
                one_segment_mut(&mut distance_sum)?,
            );
            // A node left out keeps an older word in `next`; its bits reached all its
            // out-neighbours two or more steps ago, so they find nothing new there.
            let counts = inn.scan(
                out,
                |v| at[v].load(Relaxed) != full,
                |c: &mut PerSource, (next, sum), i, row, _| {
                    let seen = at[row.node].load(Relaxed);
                    let new = row.list.iter().fold(0, |x, &u| x | f[u]) & !seen;
                    next[i] = new;
                    if new != 0 {
                        at[row.node].store(seen | new, Relaxed);
                        sum[i] += u64::from(level) * u64::from(new.count_ones());
                        let mut bits = new;
                        while bits != 0 {
                            c.0[bits.trailing_zeros() as usize] += 1;
                            bits &= bits - 1;
                        }
                    }
                },
            )?;
            if counts.0.iter().all(|&c| c == 0) {
                break;
            }
            add_level(
                &mut levels,
                usize_of(level),
                first,
                &counts.0[..batch.len()],
            );
            std::mem::swap(&mut frontier, &mut next);
        }
        inn.vertices(one_segment_mut(&mut reached)?, |(): &mut (), r, i, v, _| {
            r[i] += at[v].load(Relaxed).count_ones();
        })?;
    }
    for l in &mut levels {
        l.extend(std::iter::repeat_n(0, sources.len() - l.len()));
    }
    Ok(MultiHops {
        levels,
        distance_sum,
        reached,
    })
}

/// Writes the counts of the batch starting at source `first` into level `d`.
fn add_level(levels: &mut Vec<Vec<u64>>, d: usize, first: usize, counts: &[u64]) {
    while levels.len() <= d {
        levels.push(Vec::new());
    }
    let l = &mut levels[d];
    if l.len() < first + counts.len() {
        l.extend(std::iter::repeat_n(0, first + counts.len() - l.len()));
    }
    l[first..first + counts.len()].copy_from_slice(counts);
}
