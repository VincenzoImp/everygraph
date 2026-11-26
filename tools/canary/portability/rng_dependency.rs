// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: entropy-seeded and version-dependent generators (rule R9 of
// docs/research/sota-numerics.md). The paths of the list resolve once `rand` is a
// dependency; `cargo deny` refuses the dependency itself.
//
// dependency: rand = { version = "0.9", default-features = false, features = ["std", "std_rng", "thread_rng", "os_rng"] }
// check: clippy
// crates: everygraph-cli everygraph-community everygraph-core everygraph-format everygraph-ingest everygraph-io everygraph-kernels everygraph-map everygraph-py everygraph-query everygraph-wasm
// expect: use of a disallowed method `rand::rng`
// expect: use of a disallowed type `rand::rngs::StdRng`
// expect: use of a disallowed type `rand_core::OsRng`

//! Numerics canary probe (never part of a build of the real crate).

use rand::{RngCore, SeedableRng, TryRngCore};

/// Draws from the thread's generator, a seeded `StdRng` and the OS.
pub fn draws() -> (u64, u64, u32) {
    let seeded = rand::rngs::StdRng::seed_from_u64(1).next_u64();
    let mut os: rand::rngs::OsRng = rand::rngs::OsRng;
    let os = os.try_next_u32().unwrap_or(0);
    (rand::rng().next_u64(), seeded, os)
}
