// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The no-OOM test suite of Everygraph (issue #53): every operation either completes
//! within its memory budget or refuses with a typed error, at every budget, and the
//! memory it holds without a reservation does not grow with the graph.
//!
//! A development crate (`publish = false`): nothing shipped depends on it. It owns:
//!
//! | Module | What it holds |
//! |---|---|
//! | [`counting`] | the counting global allocator: requested bytes, their peak, and the most bytes beyond the ledger by phase |
//! | [`ledger`] | the accountant of a measured run: a budget that tells the counter what it holds, and can refuse its k-th reservation |
//! | [`graphs`] | generated graphs: random, hubs, a long chain, small components, repeated rows, isolated nodes, sparse text ids |
//! | [`ops`] | the operations, called as the command line calls them |
//! | [`probe`] | one measured run, the slope fit, the smallest budget, the refusal sweep |
//! | [`search`] | a hill climb over graph recipes towards more untracked heap per node |
//! | [`cgroup`] | runs in a child process under `systemd-run … -p MemoryMax=…`, read from the control group |
//!
//! The tests are in `tests/`: `no_oom.rs` checks every operation in process (the PR
//! set, about two minutes; `--ignored` for the nightly sizes), `cgroup.rs` the process's
//! real anonymous memory.
//!
//! Why two measures: the counting allocator sees every byte asked for, by any code, so
//! a buffer that nobody reserved shows at any size, but not what the allocator keeps
//! after a free; the control group sees what the process really holds, but only at
//! sizes where the difference is larger than the noise (`design/DECISIONS.md`, "The
//! memory promise covers the process").

pub mod cgroup;
pub mod counting;
pub mod graphs;
pub mod ledger;
pub mod ops;
pub mod probe;
pub mod search;
