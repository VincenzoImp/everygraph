// SPDX-License-Identifier: Apache-2.0 OR MIT

//! From input data to an analysis-ready graph: text and Parquet readers, the identifier
//! dictionary, and the two-pass build pipeline. Empty until issues #32 (text reader),
//! #33 (Parquet), #34 (id dictionary) and #35 (build pipeline).

#![forbid(unsafe_code)]

pub mod text;
