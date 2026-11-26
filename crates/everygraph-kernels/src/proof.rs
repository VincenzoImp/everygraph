// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The framework's proof kernel for connected components, written once against [`Plan`]
//! and run unchanged over every [`Source`] (ADR 0012, #44).
//!
//! It is the lab's kernel (`lab/b1-core/crates/glab-core/src/kernels.rs`) with the same
//! algorithm, constants and work partition, and nothing more. The production kernel,
//! with the semantics of `docs/spec/graph-semantics.md`, is #48, which replaces it.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use everygraph_core::{Memory, Reservation};

use crate::edge_map::{Error, Node, NodeCells, Plan, Source, all};

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
        plan.scan((), all, |(): &mut (), (), _, row, _| {
            let (u, list) = (row.node, row.list);
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
            |(): &mut (), (), _, row, _| {
                let (u, list) = (row.node, row.list);
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
