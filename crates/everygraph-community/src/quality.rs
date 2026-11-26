// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The community table and the objectives (`docs/spec/graph-semantics.md` § 5.5, S5):
//!
//! ```text
//! Q   = Σ_c [ L(c) / W  −  γ·( S(c) / 2W )² ],          and 0 when W = 0
//! Q_d = Σ_c [ L(c) / W  −  γ·S⁺(c)·S⁻(c) / W² ]          (Leicht–Newman)
//! CPM = Σ_c [ L(c)  −  γ·n_c·(n_c − 1) / 2 ]
//! ```
//!
//! with `L(c)` the weight of the pairs inside `c` (a kept loop once), `S(c)` the sum of
//! the strengths of its nodes (a loop's `2w` included), `W` the total weight and `n_c`
//! its nodes. On the undirected projection of a directed graph the pair `{u, v}` weighs
//! `A[u][v] + A[v][u]`, so `L(c)` is also the directed weight inside `c`, and
//! `S⁺(c)·S⁻(c) = (S(c)² − D(c)²) / 4` with `D = S⁺ − S⁻`, the skew. Every sum is an
//! integer, so each value has the same bits for any order, any thread count, and on a
//! partition and on its quotient graph under singletons.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};

use everygraph_core::{Memory, par};
use everygraph_kernels::edge_map::{Error, Plan, Settings, Source, all};

#[cfg(test)]
use crate::aggregate::Quotient;
use crate::column::{Column, GRAIN};
use crate::leiden::{Objective, Options};
use crate::level::{Strengths, Weights, strengths};
use crate::weights::{EdgeWeights, Scaled, power_of_two, scaled};

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
    /// `S⁺(c) − S⁻(c)`, with directed weights only.
    pub skew: Option<Column<i64>>,
    /// The weights are in units of `2^-s` (`EdgeWeights`); `None` on an unweighted
    /// graph.
    pub weight_scale: Option<i32>,
}

impl Table {
    /// The weight of the pairs between each community and the rest of the graph:
    /// `S(c) − 2·L(c)`.
    #[must_use]
    pub fn cut(&self, c: usize) -> u64 {
        self.volume[c] - 2 * self.internal[c]
    }

    /// The modularity of the partition at resolution `gamma`: of the undirected
    /// projection for a directed graph.
    #[must_use]
    pub fn modularity(&self, gamma: f64) -> f64 {
        self.normalized(gamma, squares(&self.volume, |&s| s))
    }

    /// The directed modularity of Leicht and Newman at resolution `gamma`; `None`
    /// without directed weights.
    #[must_use]
    pub fn directed_modularity(&self, gamma: f64) -> Option<f64> {
        let skew = self.skew.as_ref()?;
        let both = squares(&self.volume, |&s| s) - squares(skew, |d| d.unsigned_abs());
        Some(self.normalized(gamma, both))
    }

    /// The Constant Potts Model at resolution `gamma`, in the units of the weights.
    #[must_use]
    pub fn cpm(&self, gamma: f64) -> f64 {
        let inside: u64 = self.internal.iter().sum();
        let pairs: u128 = par::map_ranges(self.nodes.len(), GRAIN, |r| {
            self.nodes[r]
                .iter()
                .map(|&n| u128::from(n) * u128::from(n.saturating_sub(1)) / 2)
                .sum::<u128>()
        })
        .into_iter()
        .sum();
        let unit = self.weight_scale.map_or(1.0, |s| power_of_two(-s));
        inside as f64 * unit - gamma * pairs as f64
    }

    /// The value of the objective of `options` at its resolution.
    pub(crate) fn objective(&self, options: &Options) -> f64 {
        let gamma = options.resolution;
        match options.objective {
            Objective::Cpm => self.cpm(gamma),
            Objective::Modularity => match self.directed_modularity(gamma) {
                Some(q) if options.use_direction => q,
                _ => self.modularity(gamma),
            },
        }
    }

    /// `2·L / 2W − γ·squares / (2W)²`, where `squares` stands for `Σ_c S(c)²`.
    fn normalized(&self, gamma: f64, squares: u128) -> f64 {
        let total: u64 = self.volume.iter().sum();
        if total == 0 {
            return 0.0;
        }
        let inside: u64 = self.internal.iter().sum();
        let total = total as f64;
        2.0 * inside as f64 / total - gamma * (squares as f64 / (total * total))
    }
}

/// `Σ x²` over `values`, exactly.
fn squares<T: Sync>(values: &[T], magnitude: impl Fn(&T) -> u64 + Sync) -> u128 {
    par::map_ranges(values.len(), GRAIN, |r| {
        values[r]
            .iter()
            .map(|x| u128::from(magnitude(x)) * u128::from(magnitude(x)))
            .sum::<u128>()
    })
    .into_iter()
    .sum()
}

/// The table of the partition `map` (values below `count`) of a level, whose weights
/// have the scale `weight_scale`. With `k`, the nodes of a community are the sum of its
/// nodes' node counts when `k` has them (a coarse level of the Constant Potts Model),
/// and its skew the sum of theirs.
pub(crate) fn tally<S: Source>(
    plan: &Plan<'_, '_, S>,
    memory: &Memory,
    w: Weights<'_>,
    k: Option<&Strengths>,
    weight_scale: Option<i32>,
    map: &[u32],
    count: u32,
) -> Result<Table, Error> {
    let sizes = k.and_then(|k| k.mass.as_deref());
    let skew = k.and_then(|k| k.skew.as_deref());
    let count = count as usize;
    let nodes = Column::new(memory, "community nodes", count, |_| AtomicU32::new(0))?;
    let internal = Column::new(memory, "community internal weight", count, |_| {
        AtomicU64::new(0)
    })?;
    let volume = Column::new(memory, "community volumes", count, |_| AtomicU64::new(0))?;
    let skews = Column::new(memory, "community skews", skew.map_or(0, |_| count), |_| {
        AtomicI64::new(0)
    })?;
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
        let size = sizes.map_or(1, |n| {
            u32::try_from(n[row.node.index()]).expect("below the node count")
        });
        nodes[c].fetch_add(size, Relaxed);
        internal[c].fetch_add(inside, Relaxed);
        volume[c].fetch_add(s, Relaxed);
        if let Some(d) = skew {
            skews[c].fetch_add(d[row.node.index()], Relaxed);
        }
    })?;
    Ok(Table {
        nodes: nodes.map(AtomicU32::into_inner),
        internal: internal.map(|x| x.into_inner() / 2),
        volume: volume.map(AtomicU64::into_inner),
        skew: skew.map(|_| skews.map(AtomicI64::into_inner)),
        weight_scale,
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
    weighted_table(source, EdgeWeights::Unit, memory, settings, membership)
}

/// The community table of a partition, as [`table`], of a graph with `weights`: its
/// weights in units of `2^-s` for the scale `s` that [`leiden_weighted`] would choose
/// (`weight_scale`), and with directed weights the skews. The weights must agree across
/// the two entries of each pair, as for [`leiden_weighted`].
///
/// # Errors
///
/// As [`table`]; [`Error::Weight`] and [`Error::Invalid`] for weights
/// [`leiden_weighted`] refuses.
///
/// [`leiden_weighted`]: crate::leiden_weighted
pub fn weighted_table<S: Source>(
    source: &S,
    weights: EdgeWeights<'_>,
    memory: &Memory,
    settings: Settings,
    membership: &[u32],
) -> Result<Table, Error> {
    let count = communities(source.nodes(), membership)?;
    let Scaled {
        w,
        scale: weight_scale,
        ..
    } = scaled(source, weights)?;
    let directed = Options {
        use_direction: true,
        ..Options::default()
    };
    Plan::with(source, memory, settings, |plan| {
        let k = match w {
            Weights::Directed { .. } => Some(strengths(&plan, memory, w, &directed)?),
            _ => None,
        };
        tally(
            &plan,
            memory,
            w,
            k.as_ref(),
            weight_scale,
            membership,
            count,
        )
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

#[cfg(test)]
impl Quotient {
    /// The community table of the quotient's nodes, each alone: equal to the table of
    /// the partition it was made from, row for row.
    pub(crate) fn table(&self, memory: &Memory, settings: Settings) -> Result<Table, Error> {
        let csr = self.csr()?;
        let n = self.nodes();
        let single = Column::new(memory, "membership", n as usize, |v| {
            u32::try_from(v).expect("a node id")
        })?;
        Plan::with(&csr, memory, settings, |plan| {
            tally(
                &plan,
                memory,
                Weights::Entries(&self.weights),
                None,
                None,
                &single,
                n,
            )
        })
    }
}
