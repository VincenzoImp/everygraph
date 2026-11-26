// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The correctness oracle of Everygraph (issue #6): everything needed to show that a
//! kernel computes the right answer, and the same answer under every configuration.
//!
//! A development crate (`publish = false`). Nothing in a shipped artifact depends on it;
//! it may use files, threads and processes, and it is not under the portability rule of
//! `everygraph-core`.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`generators`], [`corpus`] | deterministic graph generators and the fixed CI and nightly corpora |
//! | [`strategies`] | `proptest` strategies over the generators |
//! | [`exchange`] | the on-disk exchange form shared with Python and external programs |
//! | [`projection`] | the analytic graph: the semantics of `m2-09` applied to the events |
//! | [`kernel`] | the kernels that can be asked for and the canonical form of answers |
//! | [`reference`](mod@reference) | sequential in-process reference kernels |
//! | [`compare`] | comparators and tolerances |
//! | [`invariants`] | checks that need no reference |
//! | [`plugin`] | the [`KernelUnderTest`] trait and the harness |
//! | [`external`] | the protocol for kernels in another process (the lab prototypes) |
//! | [`toy`] | a toy kernel with seeded bugs, the proof that the checks catch them |
//! | [`golden`] | known answers on small real graphs, with provenance |
//! | [`ldbc`] | LDBC Graphalytics data sets, the six algorithms as its specification defines them, its match rules |
//!
//! The Python references (NetworkX, python-igraph, scipy) live in `oracle/` at the root of
//! the repository; `oracle/README.md` describes the whole workflow and
//! `oracle/SEMANTICS.md` where the reference libraries disagree and what Everygraph does.
//!
//! # Plugging in a kernel (issue #45 onwards)
//!
//! A kernel crate adds `everygraph-oracle` as a dev-dependency and, in a test, implements
//! [`KernelUnderTest`] for its kernel: `supports` names the kernels and graph kinds it
//! handles, `configurations` lists what may not change the answer (threads, chunk size,
//! storage, regime), `run` returns the result in canonical form. Then:
//!
//! ```
//! use everygraph_oracle::corpus::{Tier, entries};
//! use everygraph_oracle::plugin::{Options, Prepared, run};
//! use everygraph_oracle::toy::Toy; // stands in for the kernel under test
//!
//! let graphs: Vec<Prepared> = entries(Tier::Ci).into_iter().take(20).map(Prepared::new).collect();
//! let report = run(&Toy::default(), &graphs, Options::default());
//! assert!(report.findings.is_empty(), "{:?}", report.findings);
//! ```
//!
//! That checks the kernel against the in-process references, the invariants, and
//! identity across configurations. The comparison with NetworkX, python-igraph and scipy
//! is added by loading the graphs with [`plugin::Prepared::load`] from a corpus directory
//! that `oracle-ref run` has filled (`oracle/run-ci.sh` does both); property-based tests
//! use [`strategies`] and [`plugin::Prepared::new`] on each generated graph.
//!
//! Two things a real kernel needs that the toy does not: the configuration's thread count
//! must be honoured (the toy starts its own threads; an engine kernel runs on
//! `everygraph_core::par`, whose pool size a test must be able to set), and the storage
//! and regime axes mean something only for a kernel that runs on the codec (#30) and the
//! I/O backends (#29). A kernel declares the configurations it has.

#![forbid(unsafe_code)]

pub mod compare;
pub mod corpus;
pub mod exchange;
pub mod external;
pub mod generators;
pub mod golden;
pub mod graph;
pub mod invariants;
pub mod kernel;
pub mod ldbc;
pub mod plugin;
pub mod projection;
pub mod reference;
pub mod rng;
pub mod strategies;
pub mod toy;

pub use kernel::{Kernel, Output};
pub use plugin::KernelUnderTest;

/// A `u32` node id as an index.
///
/// # Panics
///
/// On a target where `usize` is narrower than 32 bits.
#[must_use]
pub fn ix(v: u32) -> usize {
    usize::try_from(v).expect("usize holds u32")
}

/// A `u64` count or offset as an index.
///
/// # Panics
///
/// If it does not fit (only possible on a 32-bit target with a huge graph).
#[must_use]
pub fn to_usize(x: u64) -> usize {
    usize::try_from(x).expect("offset fits in usize")
}

/// A length as `u64`.
///
/// # Panics
///
/// On a target where `usize` is wider than 64 bits.
#[must_use]
pub fn to_u64(x: usize) -> u64 {
    u64::try_from(x).expect("u64 holds usize")
}

/// An integer below 2^53 as the exact double.
///
/// # Panics
///
/// If `x >= 2^53`.
#[must_use]
pub fn exact_f64(x: u64) -> f64 {
    assert!(x < (1 << 53), "integer {x} is not exact in a double");
    let hi = u32::try_from(x >> 32).expect("high half");
    let lo = u32::try_from(x & 0xFFFF_FFFF).expect("low half");
    f64::from(hi) * 4_294_967_296.0 + f64::from(lo)
}
