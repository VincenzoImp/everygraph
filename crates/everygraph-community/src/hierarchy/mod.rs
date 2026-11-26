// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The community hierarchy kept in the graph file (`docs/spec/eg-communities.md`): every
//! level of a [`Communities`](crate::Communities) result with exact counts and its
//! quotient graph, stored as one result with parts (`docs/spec/eg-container.md` § 6).
//!
//! - [`store`] writes it level by level, from the finest: each quotient graph is built
//!   from the one below, written as a compressed adjacency whose events are its weights,
//!   and dropped, so at most two levels are in memory at once.
//! - [`Hierarchy`] reads it: membership at any level, the community table, parents,
//!   ancestors and descendants, the members of a community, the quotient graphs.
//! - [`check`] is what `everygraph verify --deep` runs on it: the conservation of node
//!   counts, volumes and internal weights across levels and against the graph.
//!
//! Levels are numbered from 1 at the coarsest; communities at every level from 0 by
//! decreasing node count, then by their smallest node (`docs/api-sketch.md` § 2.12). The
//! numbering is a function of the partition, so a deterministic run (DESIGN D14) gives
//! every community the same id on every run.

mod check;
mod read;
pub(crate) mod write;

use std::error::Error as StdError;
use std::fmt;

use everygraph_format::adjacency::{EncodeError, FormatError};
use everygraph_format::container::{ContainerError, GraphIdentity, ResultKey};
use everygraph_kernels::edge_map::Error;

pub use check::check;
pub use read::{Hierarchy, StoredQuotient};
pub use write::{store, store_levels};

use crate::leiden::{Iterations, Mode, Objective, Options, Refinement, SplitAt};

/// The operation of the result (`results/communities/<key>`).
pub const OPERATION: &str = "communities";
/// The version of the layout and of the algorithm that fills it, in the key.
pub const VERSION: u32 = 1;

/// The parts of a level `level` (`<level>/nodes` and so on), and of the result.
fn part(level: u32, name: &str) -> String {
    format!("{level}/{name}")
}
const MEMBERSHIP: &str = "membership";
const NODES: &str = "nodes";
const INTERNAL: &str = "internal";
const VOLUME: &str = "volume";
const PARENT: &str = "parent";
const QUOTIENT: &str = "quotient";

/// The key of the hierarchy of a Leiden run with `options` on the graph `graph`: every
/// option that can change the result, none that cannot.
///
/// # Errors
///
/// None in practice: the names are valid.
pub fn key(graph: GraphIdentity, options: &Options) -> Result<ResultKey, ContainerError> {
    let iterations = match options.iterations {
        Iterations::Fixed(k) => k.to_string(),
        Iterations::Converge { max } => format!("converge {max}"),
    };
    let refinement = match options.refinement {
        Refinement::Greedy => "greedy".to_owned(),
        Refinement::Randomized { theta } => format!("randomized {theta}"),
        Refinement::Louvain => "louvain".to_owned(),
    };
    let objective = match options.objective {
        Objective::Modularity => "modularity",
        Objective::Cpm => "cpm",
    };
    let split = match options.split {
        SplitAt::Last => "last",
        SplitAt::EveryLevel => "every level",
    };
    ResultKey::new(
        graph,
        OPERATION,
        VERSION,
        options.mode == Mode::Deterministic,
    )?
    .with("method", "leiden")?
    .with("objective", objective)?
    .with("use_direction", &options.use_direction.to_string())?
    .with("iterations", &iterations)?
    .with("resolution", &options.resolution.to_string())?
    .with("seed", &options.seed.to_string())?
    .with("refinement", &refinement)?
    .with("prolong", &options.prolong.to_string())?
    .with("split", split)
}

/// Why a hierarchy could not be stored or read.
#[derive(Debug)]
#[non_exhaustive]
pub enum HierarchyError {
    /// The graph could not be read, or a buffer did not fit the budget.
    Graph(Error),
    /// The file could not be read or written.
    File(ContainerError),
    /// The communities or the stored hierarchy break a rule: one sentence.
    Invalid(String),
    /// An argument out of range: a level that does not exist, a community beyond the
    /// count.
    Argument(&'static str),
}

impl fmt::Display for HierarchyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(e) => e.fmt(f),
            Self::File(e) => e.fmt(f),
            Self::Invalid(what) => f.write_str(what),
            Self::Argument(what) => f.write_str(what),
        }
    }
}

impl StdError for HierarchyError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Graph(e) => Some(e),
            Self::File(e) => Some(e),
            Self::Invalid(_) | Self::Argument(_) => None,
        }
    }
}

impl From<Error> for HierarchyError {
    fn from(e: Error) -> Self {
        Self::Graph(e)
    }
}

impl From<everygraph_core::MemoryError> for HierarchyError {
    fn from(e: everygraph_core::MemoryError) -> Self {
        Self::Graph(e.into())
    }
}

impl From<ContainerError> for HierarchyError {
    fn from(e: ContainerError) -> Self {
        Self::File(e)
    }
}

impl From<FormatError> for HierarchyError {
    fn from(e: FormatError) -> Self {
        Self::Invalid(e.to_string())
    }
}

impl From<EncodeError> for HierarchyError {
    fn from(e: EncodeError) -> Self {
        match e {
            EncodeError::OutOfMemory { bytes } => Self::Graph(Error::Memory(
                everygraph_core::MemoryError::AllocationFailed {
                    purpose: "quotient graph",
                    bytes,
                },
            )),
            other => Self::Invalid(other.to_string()),
        }
    }
}
