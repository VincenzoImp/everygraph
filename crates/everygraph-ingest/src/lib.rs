// SPDX-License-Identifier: Apache-2.0 OR MIT

//! From input data to an analysis-ready graph. What lives here: the text reader
//! ([`text`], issue #32), the identifier dictionary ([`ids`], issue #34) and the graph
//! build ([`build`], issue #35), which writes the sections of a `.eg` graph: each
//! adjacency as pairs and, when some pair repeats, its `.events` section (ADR 0028).
//! [`graph`] opens what the build wrote. With the feature `native`, `native` makes a
//! graph file from a file: scratch directory, output name, and a file never left
//! half-written. Still to come: the Parquet reader (#33).

#![forbid(unsafe_code)]

pub mod build;
pub mod graph;
pub mod ids;
#[cfg(feature = "native")]
pub mod native;
pub mod text;
