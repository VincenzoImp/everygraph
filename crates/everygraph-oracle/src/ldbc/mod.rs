// SPDX-License-Identifier: Apache-2.0 OR MIT

//! LDBC Graphalytics as an oracle for large graphs (issue #177).
//!
//! LDBC Graphalytics (specification v1.0.5, arXiv 2011.15028 v6) publishes graphs up to
//! billions of edges together with the reference output of six deterministic algorithms.
//! This module reads a data set in its "EVLP" form (`<g>.properties`, `<g>.v`, `<g>.e`
//! and one `<g>-<ALGO>` file per algorithm), implements the six algorithms as the
//! specification defines them ([`spec`]), and compares any result with the reference
//! output under the specification's match rules ([`validate`]).
//!
//! Vertex ids are mapped to dense indices **in increasing id order**. So the smallest
//! index is the smallest id, and the id-dependent rules (CDLP's initial labels and ties,
//! smallest-id component labels) need no translation.
//!
//! Results checked this way are not LDBC Benchmark Results: the name is reserved for
//! audited runs (LDBC fair-use policy).

pub mod read;
pub mod run;
pub mod spec;
pub mod validate;

pub use read::{Csr, Dataset, Ids, LdbcGraph, Lists};

/// The six algorithms of Graphalytics (specification § 2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Algorithm {
    /// Breadth-first search: depth from the source; unreachable is `i64::MAX`.
    Bfs,
    /// PageRank: fixed iterations, sink mass spread uniformly, f64.
    Pr,
    /// Weakly connected components: labels up to relabelling.
    Wcc,
    /// Community detection by synchronous label propagation.
    Cdlp,
    /// Local clustering coefficient, the directed definition included.
    Lcc,
    /// Single-source shortest paths over the edge weights; unreachable is infinity.
    Sssp,
}

/// How a result is compared with the reference output (specification § 2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Every value identical.
    Exact,
    /// The same partition of the vertices, whatever the label values.
    Equivalence,
    /// `|r - s| <= 1e-4 |r|`, `r` the reference.
    Epsilon,
}

impl Algorithm {
    /// All six, in the specification's order.
    pub const ALL: [Self; 6] = [
        Self::Bfs,
        Self::Pr,
        Self::Wcc,
        Self::Cdlp,
        Self::Lcc,
        Self::Sssp,
    ];

    /// The suffix of the reference output file, `<g>-<SUFFIX>`.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Bfs => "BFS",
            Self::Pr => "PR",
            Self::Wcc => "WCC",
            Self::Cdlp => "CDLP",
            Self::Lcc => "LCC",
            Self::Sssp => "SSSP",
        }
    }

    /// The lowercase name used in `.properties` files and on the command line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bfs => "bfs",
            Self::Pr => "pr",
            Self::Wcc => "wcc",
            Self::Cdlp => "cdlp",
            Self::Lcc => "lcc",
            Self::Sssp => "sssp",
        }
    }

    /// The algorithm named `name` ([`Algorithm::name`]).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    /// The match rule of the specification.
    #[must_use]
    pub const fn rule(self) -> Rule {
        match self {
            Self::Bfs | Self::Cdlp => Rule::Exact,
            Self::Wcc => Rule::Equivalence,
            Self::Pr | Self::Lcc | Self::Sssp => Rule::Epsilon,
        }
    }
}

/// One value per vertex, in dense index order.
#[derive(Clone, Debug, PartialEq)]
pub enum Values {
    /// BFS depths (unreachable: [`UNREACHABLE`]), CDLP and WCC labels (vertex ids).
    Ints(Vec<u64>),
    /// PageRank, LCC and SSSP values (unreachable: infinity).
    Reals(Vec<f64>),
}

/// The BFS depth of an unreachable vertex: `i64::MAX`, as the specification writes it.
pub const UNREACHABLE: u64 = 9_223_372_036_854_775_807;
