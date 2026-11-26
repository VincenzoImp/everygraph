// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Weakly connected components (#48): `ConnectIt`'s sampling and union-find finish
//! (Dhulipala, Hong and Shun, VLDB 2021), and the numbering of components that results
//! show (`docs/api-sketch.md` § 2.12).
//!
//! **Sampling** (k-out, hybrid). Each node is joined to its first neighbour and to one
//! neighbour drawn by a counter-based hash of the node, and the forest is flattened. The
//! most frequent label among 1,024 drawn nodes is then the giant component. First
//! neighbours alone ("kout-afforest", the lab's Afforest) fail on graphs whose lists are
//! in a locally clustered order, as a community order makes them: the first neighbour
//! is then in the node's own cluster (`ConnectIt` § 5.2; `docs/research/sota-kernels.md`,
//! challenge 6). Sampling is skipped when there are fewer than three entries per node.
//!
//! **Finish** (UF-Rem-CAS with splicing). Every list is joined, except, on a symmetric
//! graph, the lists of the giant component's nodes: an edge from the giant component to
//! another node is in the other node's list too.
//!
//! **Labels.** A parent is never larger than its child, so a root is the smallest node of
//! its tree, and after the final flattening every node holds the smallest node of its
//! component, whatever order the joins land in (DESIGN D14). The sampling changes only
//! the work.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use everygraph_core::{Memory, MemoryKind, SegVec, par};

use crate::Atomics;
use crate::edge_map::{Error, Node, NodeCells, Plan, Source, ix};
use crate::rng::{below, draw};

/// Nodes per task of the passes outside the drivers.
const GRAIN: NonZeroUsize = NonZeroUsize::new(1 << 14).expect("non-zero");

/// The key of the sampling's draws. Labels do not depend on it.
const SEED: u64 = 0x5743_4320_6b6f_7574;

/// How [`wcc`] reads the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WccOptions {
    /// Every list holds the node's neighbours in both directions (an undirected graph, or
    /// the undirected projection of a directed one). If false, the lists are out-lists of
    /// a directed graph and every list is read in the finish.
    pub symmetric: bool,
    /// Sample before the finish. False makes one pass over each list, which is what a
    /// streamed source wants (the semi-external regime).
    pub sample: bool,
}

impl Default for WccOptions {
    /// Symmetric lists, sampled.
    fn default() -> Self {
        Self {
            symmetric: true,
            sample: true,
        }
    }
}

/// Bytes of state that [`wcc`] reserves for a graph of `nodes` nodes: one parent each.
#[must_use]
pub const fn wcc_state_bytes(nodes: u32) -> u64 {
    nodes as u64 * 4
}

/// Joins the sets of `x` and `y`: Rem's algorithm with splicing, lock-free by CAS
/// (Patwary, Blair and Manne, SEA 2010; `ConnectIt`'s UF-Rem-CAS). Each parent only goes
/// down, so a root is the smallest node of its tree.
#[inline]
fn unite<'g>(parent: &NodeCells<'g>, mut x: Node<'g>, mut y: Node<'g>) {
    loop {
        let (mut px, mut py) = (parent.load(x, Relaxed), parent.load(y, Relaxed));
        if px == py {
            return;
        }
        if px < py {
            (x, y, px, py) = (y, x, py, px);
        }
        if x == px {
            if parent.compare_exchange(x, x, py, Relaxed).is_ok() {
                return;
            }
        } else {
            let _ = parent.compare_exchange(x, px, py, Relaxed);
            x = px;
        }
    }
}

/// The most frequent label among 1,024 drawn nodes; of equally frequent ones the
/// smallest.
fn giant<'g, S: Source>(plan: &Plan<'_, 'g, S>, parent: &NodeCells<'g>) -> Option<Node<'g>> {
    let n = u64::from(plan.nodes());
    let mut labels: Vec<Node<'g>> = (0..1024)
        .filter_map(|i| {
            let v = u32::try_from(below(draw(SEED, *b"wccg", [i, 0, 0]), n)).ok()?;
            Some(parent.load(plan.node(v)?, Relaxed))
        })
        .collect();
    labels.sort_unstable();
    labels
        .chunk_by(|a, b| a == b)
        .max_by_key(|run| (run.len(), std::cmp::Reverse(run[0])))
        .map(|run| run[0])
}

/// Weakly connected components: `labels[v]` is the smallest node of the component of
/// `v`. Returned as the atomics they were computed in, with the reservation that holds
/// them ([`wcc_state_bytes`]).
///
/// Lists may hold the node itself (a kept loop), which joins nothing.
///
/// # Errors
///
/// From the plan's source, or the labels do not fit in the budget.
pub fn wcc<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    options: WccOptions,
) -> Result<Atomics, Error> {
    let parent = plan.node_cells(memory)?;
    let mut skip = None;
    if options.sample && plan.source().entries() >= 3 * u64::from(plan.nodes()) {
        plan.scan(
            (),
            |_| true,
            |(): &mut (), (), _, row, _| {
                let (u, list) = (row.node, row.list);
                let Some(&first) = list.first() else {
                    return;
                };
                unite(&parent, u, first);
                if let Some(rest) = list.len().checked_sub(1).filter(|&r| r > 0) {
                    let at = below(draw(SEED, *b"wcck", [u.get(), 0, 0]), rest as u64);
                    unite(&parent, u, list[1 + ix(at)]);
                }
            },
        )?;
        parent.flatten();
        if options.symmetric {
            skip = giant(plan, &parent);
        }
    }
    plan.scan(
        (),
        |u| skip.is_none_or(|g| parent.load(u, Relaxed) != g),
        |(): &mut (), (), _, row, _| {
            for &v in row.list {
                unite(&parent, row.node, v);
            }
        },
    )?;
    parent.flatten();
    Ok(parent.into_ids())
}

/// Components numbered as results show them.
#[derive(Debug)]
pub struct Numbered {
    /// The number of each node's component: 0 for the largest; of two of equal size,
    /// the one with the earlier node comes first (`docs/api-sketch.md` § 2.12).
    pub component: SegVec<u32>,
    /// The number of nodes of each component, by number: not increasing.
    pub sizes: SegVec<u32>,
}

/// Numbers the components of `labels`, where `labels[v]` is the smallest node of the
/// component of `v` (as [`wcc`] gives them).
///
/// # Errors
///
/// [`Error::Invalid`] if a label is not the smallest node of a component that holds
/// itself; [`Error::Memory`] if the result does not fit in the budget.
///
/// # Panics
///
/// Never: sizes and numbers are counted before they are used as positions.
pub fn number(labels: &[AtomicU32], memory: &Memory) -> Result<Numbered, Error> {
    let n = labels.len();
    let invalid = Error::Invalid("component labels: not the smallest node of each component");
    let label = |v: usize| ix(labels[v].load(Relaxed).into());
    let bad = par::map_ranges(n, GRAIN, |r| {
        r.into_iter()
            .any(|v| label(v) > v || label(label(v)) != label(v))
    });
    if bad.into_iter().any(|b| b) {
        return Err(invalid);
    }
    // Sizes, by root. Runs of equal labels are added at once, so the giant component's
    // counter is not taken once per node.
    let _held = memory.reserve(
        "component sizes by node",
        MemoryKind::Anonymous,
        n as u64 * 4,
    )?;
    let size: Vec<AtomicU32> = par::collect(n, GRAIN, |_| AtomicU32::new(0));
    par::for_each_range(n, GRAIN, |r| {
        let mut run = (usize::MAX, 0);
        for v in r {
            let l = label(v);
            if l != run.0 {
                if run.1 > 0 {
                    size[run.0].fetch_add(run.1, Relaxed);
                }
                run = (l, 0);
            }
            run.1 += 1;
        }
        if run.1 > 0 {
            size[run.0].fetch_add(run.1, Relaxed);
        }
    });
    // Rank by decreasing size, then by root: the components of size `s` start after all
    // larger ones, and take their numbers in root order.
    let size_of = |v: usize| ix(size[v].load(Relaxed).into());
    let largest = (0..n)
        .filter(|&v| label(v) == v)
        .map(size_of)
        .max()
        .unwrap_or(0);
    let mut start = SegVec::try_with_len(memory, "components by size", largest + 2, 0_u32)?;
    let mut count = 0_u32;
    for v in (0..n).filter(|&v| label(v) == v) {
        *start.get_mut(size_of(v)).expect("a size up to the largest") += 1;
        count += 1;
    }
    let mut above = 0;
    for s in (0..=largest).rev() {
        let here = start.get_mut(s).expect("a size up to the largest");
        (*here, above) = (above, above + *here);
    }
    let mut sizes = SegVec::try_with_len(memory, "component sizes", ix(count.into()), 0_u32)?;
    for v in (0..n).filter(|&v| label(v) == v) {
        let s = size_of(v);
        let next = start.get_mut(s).expect("a size up to the largest");
        *sizes.get_mut(ix((*next).into())).expect("a component") =
            u32::try_from(s).expect("a size below 2^32");
        size[v].store(*next, Relaxed);
        *next += 1;
    }
    let mut component = SegVec::try_with_len(memory, "component numbers", n, 0_u32)?;
    component.for_each_chunk_mut(GRAIN, |first, out| {
        for (i, c) in out.iter_mut().enumerate() {
            *c = size[label(first + i)].load(Relaxed);
        }
    });
    Ok(Numbered { component, sizes })
}
