// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The library half of `everygraph-bench`: the dataset registry and fetcher today, the
//! benchmark harness (issue #4) later. Other tools (the harness, the correctness oracle of
//! issue #6) find datasets by name through [`datasets`].

#![forbid(unsafe_code)]

pub mod datasets;
