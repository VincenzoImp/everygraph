// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The community table and modularity (`docs/spec/graph-semantics.md` § 5.5, S5):
//!
//! ```text
//! Q = Σ_c [ L(c) / W  −  γ·( S(c) / 2W )² ],    and 0 when W = 0
//! ```
//!
//! with `L(c)` the weight of the pairs inside `c` (a kept loop once), `S(c)` the sum of
//! the strengths of its nodes (a loop's `2w` included) and `W` the total weight. Every
//! sum is an integer, so `Q` has the same bits for any order, any thread count, and on
//! a partition and on its quotient graph under singletons.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU32, AtomicU64};

use everygraph_core::{Memory, par};
use everygraph_kernels::edge_map::{Error, Plan, Settings, Source, all};

use crate::aggregate::Quotient;
use crate::column::{Column, GRAIN};
use crate::level::Weights;

/// One row per community of a partition: what `c.table()` shows.
#[derive(Debug)]
pub struct Table {
    /// Nodes in each community.
    pub nodes: Column<u32>,
    /// Weight of the pairs inside each community, a kept loop once: `L(c)`. On an
    /// unweighted graph, the number of edges inside.
    pub internal: Column<u64>,
    /// Sum of the strengths of its nodes, a kept loop counted twice: `S(c)`. On an
    /// unweighted graph, the sum of the degrees.
    pub volume: Column<u64>,
}

impl Table {
    /// The weight of the pairs between each community and the rest of the graph:
    /// `S(c) − 2·L(c)`.
    #[must_use]
    pub fn cut(&self, c: usize) -> u64 {
        self.volume[c] - 2 * self.internal[c]
    }

    /// The modularity of the partition at resolution `gamma`.
    #[must_use]
    pub fn modularity(&self, gamma: f64) -> f64 {
        let total: u64 = self.volume.iter().sum();
        if total == 0 {
            return 0.0;
        }
        let inside: u64 = self.internal.iter().sum();
        let squares: u128 = par::map_ranges(self.volume.len(), GRAIN, |r| {
            self.volume[r]
                .iter()
                .map(|&s| u128::from(s) * u128::from(s))
                .sum::<u128>()
        })
        .into_iter()
        .sum();
        let total = total as f64;
        2.0 * inside as f64 / total - gamma * (squares as f64 / (total * total))
    }
}

/// The table of the partition `map` (values below `count`) of a level.
pub(crate) fn tally<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    w: Weights<'_>,
    map: &[u32],
    count: u32,
) -> Result<Table, Error> {
    let count = count as usize;
    let nodes = Column::new(memory, "community nodes", count, |_| AtomicU32::new(0))?;
    let internal = Column::new(memory, "community internal weight", count, |_| {
        AtomicU64::new(0)
    })?;
    let volume = Column::new(memory, "community volumes", count, |_| AtomicU64::new(0))?;
    plan.scan((), all, |(): &mut (), (), _, row, _| {
        let c = map[row.node.index()];
        let (mut s, mut inside) = (0, 0);
        w.each(&row, |v, x| {
            s += x;
            if map[v.index()] == c {
                inside += x;
            }
        });
        let c = c as usize;
        nodes[c].fetch_add(1, Relaxed);
        internal[c].fetch_add(inside, Relaxed);
        volume[c].fetch_add(s, Relaxed);
    })?;
    Ok(Table {
        nodes: nodes.map(AtomicU32::into_inner),
        internal: internal.map(|x| x.into_inner() / 2),
        volume: volume.map(AtomicU64::into_inner),
    })
}

/// Checks a membership column: one value per node, each below the node count. Returns
/// the number of communities it may name (the largest value + 1).
pub(crate) fn communities(n: u32, membership: &[u32]) -> Result<u32, Error> {
    if membership.len() != n as usize {
        return Err(Error::Invalid("membership: not one value per node"));
    }
    let largest = par::map_ranges(membership.len(), GRAIN, |r| {
        membership[r].iter().copied().max()
    })
    .into_iter()
    .flatten()
    .max();
    match largest {
        Some(c) if c >= n => Err(Error::Invalid(
            "membership: a value is not below the node count",
        )),
        Some(c) => Ok(c + 1),
        None => Ok(0),
    }
}

/// The community table of a partition of an unweighted graph: `membership[v]` is the
/// community of node `v`, below the node count. Communities that no node names get a
/// row of zeros.
///
/// # Errors
///
/// [`Error::Invalid`] if `membership` does not have one value per node or names a
/// community not below the node count; the source's errors; the budget's refusal.
pub fn table<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    membership: &[u32],
) -> Result<Table, Error> {
    let count = communities(source.nodes(), membership)?;
    Plan::with(source, memory, settings, |plan| {
        tally(&plan, memory, Weights(None), membership, count)
    })
}

/// The modularity at resolution `gamma` of a partition of an unweighted graph, as
/// [`table`] takes it.
///
/// # Errors
///
/// As [`table`].
pub fn modularity<S: Source>(
    source: &S,
    memory: &Memory,
    settings: Settings,
    membership: &[u32],
    gamma: f64,
) -> Result<f64, Error> {
    Ok(table(source, memory, settings, membership)?.modularity(gamma))
}

impl Quotient {
    /// The community table of the quotient's nodes, each alone: equal to the table of
    /// the partition it was made from, row for row.
    ///
    /// # Errors
    ///
    /// The budget's refusal.
    ///
    /// # Panics
    ///
    /// Never: a quotient has fewer than 2^32 nodes.
    pub fn table(&self, memory: &Memory, settings: Settings) -> Result<Table, Error> {
        let csr = self.csr()?;
        let n = self.nodes();
        let single = Column::new(memory, "membership", n as usize, |v| {
            u32::try_from(v).expect("a node id")
        })?;
        Plan::with(&csr, memory, settings, |plan| {
            tally(&plan, memory, Weights(Some(&self.weights)), &single, n)
        })
    }
}
