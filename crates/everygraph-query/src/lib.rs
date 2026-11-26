// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Queries: neighbours and k-hop with limits, shortest paths, and filtered views.
//!
//! What lives here:
//!
//! - [`views`]: filtered views (#93): predicates on node, pair and event columns, node
//!   sets and community membership, combined into a [`ViewSpec`] that one pass over the
//!   parent turns into an `edge_map::View`, held as a mask or materialized by the
//!   planner's rule. Every kernel runs on a view as on a graph.
//!
//! Point queries (#90) and shortest paths (#91) are not here yet.

// `forbid`: the allocation rule in `clippy.toml` cannot be allowed away (issue #31).
#![forbid(unsafe_code, clippy::disallowed_methods)]

pub mod views;

pub use views::{Column, Filtered, Predicate, Scalar, Test, Values, ViewSpec};
