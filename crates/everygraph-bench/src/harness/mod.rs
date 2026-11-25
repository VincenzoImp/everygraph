// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The benchmark harness (issue #4): declarative specs under `bench/specs/`, every run
//! under a hard memory limit in a cgroup of its own, official runs behind one lock and
//! an idle gate, cold-cache runs on evicted private copies, one JSON row per repetition,
//! and a report that never prints a number without its conditions.
//!
//! The protocol and the file formats are described in `bench/README.md`; each module
//! starts with the part it implements.

pub mod cache;
pub mod extract;
pub mod gate;
pub mod lock;
pub mod machine;
pub mod measure;
pub mod procfs;
pub mod report;
pub mod row;
pub mod run;
pub mod selftest;
pub mod spec;
pub mod sys;
pub mod template;
pub mod units;
