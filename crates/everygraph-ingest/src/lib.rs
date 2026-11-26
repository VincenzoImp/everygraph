// SPDX-License-Identifier: Apache-2.0 OR MIT

//! From input data to an analysis-ready graph. What lives here: the text reader
//! ([`text`], issue #32) and the identifier dictionary ([`ids`], issue #34). Still to
//! come: the Parquet reader (#33) and the two-pass build pipeline (#35), which writes each
//! adjacency as pairs and, when some pair repeats, its `.events` section (ADR 0028).

#![forbid(unsafe_code)]

pub mod ids;
pub mod text;
