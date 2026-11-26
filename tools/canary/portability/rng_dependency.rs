// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: an entropy-seeded and a version-dependent generator (rule R9 of
// docs/research/sota-numerics.md). The paths of the list resolve once `rand` is a
// dependency; `cargo deny` refuses the dependency itself.
//
// dependency: rand = { version = "0.9", default-features = false, features = ["std", "std_rng", "thread_rng"] }
// check: clippy
// expect: use of a disallowed method `rand::rng`
// expect: use of a disallowed type `rand::rngs::StdRng`

//! Numerics canary probe (never part of a build of the real crate).

use rand::{RngCore, SeedableRng};

/// Draws from the thread's generator and from a seeded `StdRng`.
pub fn draws() -> (u64, u64) {
    let seeded = rand::rngs::StdRng::seed_from_u64(1).next_u64();
    (rand::rng().next_u64(), seeded)
}
