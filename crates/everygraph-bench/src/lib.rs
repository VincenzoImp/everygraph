// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The library half of `everygraph-bench`: the dataset registry and fetcher
//! ([`datasets`], issue #5) and the benchmark harness ([`harness`], issue #4). Other
//! tools (the correctness oracle of issue #6) find datasets by name through
//! [`datasets`].

#![forbid(unsafe_code)]

pub mod datasets;
pub mod harness;
