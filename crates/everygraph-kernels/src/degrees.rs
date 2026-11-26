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
//! | `weighted` of a directed graph | out- plus in-adjacency, with their weights | [`Loop::Once`] each |
//!
//! A list holds one entry per pair, so a degree counts distinct neighbours. A node's own
//! entry is a kept loop, which adds 2 to the undirected degree and 1 to each directed
//! one; its weight `w` adds `2w` to the undirected strength. Weights are summed in list
//! order from `+0.0`, whatever their sign or value (§ 4.4), so a strength is the same at
//! any thread count.

use everygraph_core::{Memory, SegVec};

use crate::edge_map::{Error, Plan, Source, all};

/// How a node's own entry (a kept loop) counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loop {
    /// As any other entry: directed degrees and strengths.
    Once,
    /// Twice: the diagonal `2w` of an undirected adjacency.
    Twice,
}

fn one_segment<T: Copy>(v: &mut SegVec<T>) -> Result<&mut [T], Error> {
    v.as_mut_slice().ok_or(Error::Invalid(
        "node state longer than one segment (#m3-15)",
    ))
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
    let column = one_segment(&mut out)?;
    match own {
        Loop::Once => plan.vertices(column, |(): &mut (), out, i, _, d| out[i] = d)?,
        Loop::Twice => plan.scan(column, all, |(): &mut (), out, i, row, _| {
            let loops = row.list.iter().filter(|&&u| u == row.node).count();
            out[i] = u32::try_from(row.list.len() + loops).expect("a degree below 2^32");
        })?,
    }
    Ok(out)
}

/// The sum of `weights` over each list, a node's own entry counted as `own` says.
/// `weights` holds one value per entry, in node order.
///
/// # Errors
///
/// [`Error::Invalid`] if `weights` is shorter than the entries; from the plan's source,
/// or the column does not fit in the budget.
///
/// # Panics
///
/// If the node count does not fit in memory on this target.
pub fn strengths<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    weights: &[f64],
    own: Loop,
) -> Result<SegVec<f64>, Error> {
    if (weights.len() as u64) < plan.source().entries() {
        return Err(Error::Invalid("weights: fewer than the entries"));
    }
    let n = usize::try_from(plan.nodes()).expect("a node count in memory");
    let mut out = SegVec::try_with_len(memory, "strengths", n, 0.0_f64)?;
    plan.scan(
        one_segment(&mut out)?,
        all,
        |(): &mut (), out, i, row, _| {
            let w = &weights[row.entries()];
            let mut sum = 0.0;
            for (&u, &x) in row.list.iter().zip(w) {
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
