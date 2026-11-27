// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Point queries (#90): the neighbours of nodes, their degrees, whether two nodes are
//! joined, their common neighbours, the edges between two node sets, and the nodes
//! within `k` hops of one node, each with a `limit` on the rows it returns
//! (`docs/api-sketch.md` § 3.2, ADR 0021).
//!
//! What lives here:
//!
//! - [`Graph`]: the lists a query reads — the out-adjacency, and the in-adjacency of a
//!   directed graph — from any [`Source`](everygraph_kernels::edge_map::Source) of the
//!   edge-map framework (the compressed lists, a raw copy, a stream), with the per-entry
//!   [`Columns`] that rows report. Every source gives the same rows.
//! - [`Reader`]: one per thread. It holds the decode buffers, and every query is a
//!   method of it. Many readers query one graph at once: the graph is read only.
//! - [`ids`]: external identifiers to node ids in bulk, through the dictionary.
//!
//! **Order and semantics.** Rows follow `docs/api-sketch.md` § 2.12; kept loops and
//! repeated pairs follow `docs/spec/graph-semantics.md` § 6. A batch takes a column of
//! nodes and returns one table in the order the nodes were given, `limit` applying to
//! each node. A query runs on the calling thread, in that order, and reads only the
//! graph: its rows do not depend on the thread count by construction. A cut by `limit`
//! gives the same rows on every run; a cut by `timeout` does not.
//!
//! **Cost.** A query reads only the lists it needs. `neighbors` and `khop` decode a
//! list only as far as they read it, in parts. A cut `neighbors` on a hub costs what
//! it returns. A cut `khop` also reads the entries of the nodes it met before, which
//! can be most of a hop's lists when its nodes are well joined.
//!
//! **Memory.** Before it reads a list, a query adds up the most it can hold: its rows
//! and sets from the index, and what each source's reader takes to read the longest list
//! it may read (`Source::list_bytes`, which counts a stream's window). It is refused with
//! [`QueryError::Budget`] if the budget does not have it. Every buffer is then held
//! through the budget.
//!
//! **Time.** The crate reads no clock (it builds for wasm32): a `timeout` is a
//! [`Deadline`] that the caller implements.

// The portability and allocation lists in `clippy.toml` cannot be allowed away.
#![forbid(
    unsafe_code,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use std::error::Error as StdError;
use std::fmt;

use everygraph_core::{InsufficientBudget, MemoryError};
use everygraph_ingest::ids::DictionaryError;
use everygraph_kernels::edge_map::Error as ListError;

mod graph;
pub mod ids;
mod khop;
mod neighbors;
mod pairs;
mod seen;

pub use graph::{Columns, Graph, Reader};
pub use khop::Hops;
pub use neighbors::{Degrees, EdgeRows};
pub use pairs::Common;

/// Which lists a query follows (`docs/api-sketch.md` § 2.4). On an undirected graph
/// every value means the same: all neighbours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// Out-neighbours.
    #[default]
    Out,
    /// In-neighbours: the in-adjacency.
    In,
    /// Both: the undirected projection.
    Both,
}

/// What cut a result short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// The query reached its `limit`; the rows kept are those of § 2.12, on every run.
    Limit,
    /// The query reached its `timeout`; the rows depend on the machine's speed.
    Timeout,
}

/// A result cut short (`docs/api-sketch.md` § 2.5): `meta["truncated"]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Truncated {
    /// What cut it.
    pub by: Cut,
    /// The limit asked for.
    pub limit: u64,
    /// The rows returned.
    pub rows: u64,
    /// For `khop`, the distance of the hop it stopped in.
    pub hop: Option<u32>,
    /// The rows of the whole answer, when known without further work: a stored degree.
    pub total: Option<u64>,
}

/// When a traversal must stop: the `timeout` of `docs/api-sketch.md` § 2.4. The caller
/// reads the clock; a query asks between lists and within long ones.
pub trait Deadline {
    /// Whether the time is up.
    fn passed(&self) -> bool;
}

impl<F: Fn() -> bool> Deadline for F {
    fn passed(&self) -> bool {
        self()
    }
}

/// No timeout.
#[derive(Clone, Copy, Debug, Default)]
pub struct Never;

impl Deadline for Never {
    fn passed(&self) -> bool {
        false
    }
}

/// Why a query failed (`docs/api-sketch.md` § 2.7).
#[derive(Debug)]
#[non_exhaustive]
pub enum QueryError {
    /// A node id that is not below the number of nodes.
    UnknownNode(u32),
    /// A parameter the query cannot take: a `limit` of 0, batch columns of different
    /// lengths, columns that do not fit the lists.
    Invalid(&'static str),
    /// The query cannot be done on this graph: the reason.
    Unsupported(&'static str),
    /// The budget does not have what the query may hold; nothing was read.
    Budget(InsufficientBudget),
    /// A list could not be read, is corrupt, or a buffer did not fit while reading.
    Lists(ListError),
    /// The dictionary could not be read.
    Ids(DictionaryError),
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(v) => write!(f, "node {v} is not in the graph"),
            Self::Invalid(what) | Self::Unsupported(what) => f.write_str(what),
            Self::Budget(e) => e.fmt(f),
            Self::Lists(e) => e.fmt(f),
            Self::Ids(e) => e.fmt(f),
        }
    }
}

impl StdError for QueryError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Budget(e) => Some(e),
            Self::Lists(e) => Some(e),
            Self::Ids(e) => Some(e),
            Self::UnknownNode(_) | Self::Invalid(_) | Self::Unsupported(_) => None,
        }
    }
}

impl From<ListError> for QueryError {
    fn from(e: ListError) -> Self {
        Self::Lists(e)
    }
}

impl From<MemoryError> for QueryError {
    fn from(e: MemoryError) -> Self {
        Self::Lists(e.into())
    }
}

impl From<DictionaryError> for QueryError {
    fn from(e: DictionaryError) -> Self {
        Self::Ids(e)
    }
}

/// The checked `limit` of a query, as a row count.
fn rows_of(limit: u64) -> Result<usize, QueryError> {
    if limit == 0 {
        return Err(QueryError::Invalid("limit must be at least 1"));
    }
    Ok(usize::try_from(limit).unwrap_or(usize::MAX))
}
