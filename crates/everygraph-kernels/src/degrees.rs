// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Degree and strength columns (#50), as `docs/spec/graph-semantics.md` § 3 defines
//! them. Each is a pass over one adjacency:
//!
//! | Column | Adjacency | Own entry |
//! |---|---|---|
//! | `degree` of an undirected graph | the graph | [`Loop::Twice`] |
//! | `degree` of a directed graph | its undirected projection | [`Loop::Twice`] |
//! | `out_degree`, `in_degree` | out- or in-adjacency | [`Loop::Once`] |
//! | `weighted` of an undirected graph | the graph, with its weights | [`Loop::Twice`] |
//! | `weighted` of a directed graph | out- plus in-adjacency, with their weights, added by the caller | [`Loop::Once`] each |
//!
//! A list holds one entry per pair, so a degree counts distinct neighbours. A node's own
//! entry is a kept loop, which adds 2 to the undirected degree and 1 to each directed
//! one; its weight `w` adds `2w` to the undirected strength. Weights are summed in list
//! order from `+0.0`, whatever their sign or value (§ 4.4), so a strength is the same at
//! any thread count. A directed strength, out plus in, is rounded twice, so it may
//! differ in the last bits from a sum over the projection's pairs.

use everygraph_core::{Memory, SegVec};

use crate::edge_map::{Error, Plan, Source, all, one_segment_mut};

/// How a node's own entry (a kept loop) counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loop {
    /// As any other entry: directed degrees and strengths.
    Once,
    /// Twice: the diagonal `2w` of an undirected adjacency.
    Twice,
}

/// The number of entries of each list, a node's own entry counted as `own` says.
///
/// # Errors
///
/// From the plan's source, or the column does not fit in the budget.
///
/// # Panics
///
/// If a degree is 2^32 or more, which a graph of `u32` nodes without repeated pairs
/// cannot have.
pub fn degrees<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    own: Loop,
) -> Result<SegVec<u32>, Error> {
    let n = usize::try_from(plan.nodes()).expect("a node count in memory");
    let mut out = SegVec::try_with_len(memory, "degrees", n, 0_u32)?;
    let column = one_segment_mut(&mut out)?;
    match own {
        Loop::Once => plan.vertices(column, |(): &mut (), out, i, _, d| out[i] = d)?,
        Loop::Twice => plan.scan(column, all, |(): &mut (), out, i, row, _| {
            let loops = row.list.iter().filter(|&&u| u == row.node).count();
            out[i] = u32::try_from(row.list.len() + loops).expect("a degree below 2^32");
        })?,
    }
    Ok(out)
}

/// The sum of `weights` over each list, a node's own entry counted as `own` says;
/// without weights, every entry weighs 1. `weights` holds one value per entry, in node
/// order.
///
/// # Errors
///
/// [`Error::Invalid`] if `weights` is shorter than the entries; from the plan's source,
/// or the column does not fit in the budget.
pub fn strengths<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    weights: Option<&[f64]>,
    own: Loop,
) -> Result<SegVec<f64>, Error> {
    scaled_strengths(plan, memory, weights, 1.0, own)
}

/// [`strengths`] of the weights times `scale`, a power of two that `PageRank` takes to
/// keep its state in range; the products are exact, so the sums are those of the scaled
/// column.
pub(crate) fn scaled_strengths<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    weights: Option<&[f64]>,
    scale: f64,
    own: Loop,
) -> Result<SegVec<f64>, Error> {
    if weights.is_some_and(|w| (w.len() as u64) < plan.source().entries()) {
        return Err(Error::Invalid("weights: fewer than the entries"));
    }
    let n =
        usize::try_from(plan.nodes()).map_err(|_| Error::Invalid("a node count above memory"))?;
    let mut out = SegVec::try_with_len(memory, "strengths", n, 0.0_f64)?;
    plan.scan(
        one_segment_mut(&mut out)?,
        all,
        |(): &mut (), out, i, row, _| {
            let weight = |e: usize| weights.map_or(1.0, |w| w[e] * scale);
            let mut sum = 0.0;
            for (&u, e) in row.list.iter().zip(row.entries()) {
                let x = weight(e);
                sum += if u == row.node && own == Loop::Twice {
                    2.0 * x
                } else {
                    x
                };
            }
            out[i] = sum;
        },
    )?;
    Ok(out)
}
