// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: the allocation rule switched off by an inner `allow`. The kernel crate roots
// forbid the lint, so this is error E0453 under clippy.
//
// check: clippy
// crates: everygraph-kernels everygraph-community everygraph-map everygraph-query
// expect: error[E0453]: allow(clippy::disallowed_methods) incompatible with previous forbid

//! Allocation canary probe (never part of a build of the real crate).

/// A buffer sized outside the budget, with the lint allowed.
#[allow(clippy::disallowed_methods)]
pub fn ranks(n: usize) -> Vec<f32> {
    vec![0.0; n]
}
