// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: platform maths and `libm::pow` in a product crate (rule R2 of
// docs/research/sota-numerics.md). Every crate below has its own copy of the list or
// reads the root `clippy.toml`; each is probed.
//
// check: clippy
// crates: everygraph-cli everygraph-community everygraph-core everygraph-format everygraph-ingest everygraph-io everygraph-kernels everygraph-map everygraph-py everygraph-query everygraph-wasm
// expect: use of a disallowed method `f64::sin`
// expect: use of a disallowed method `f64::powf`
// expect: use of a disallowed method `f32::powi`
// expect: use of a disallowed method `libm::pow`
// dependency: libm = { version = "=0.2.16", default-features = false }

//! Numerics canary probe (never part of a build of the real crate).

/// Calls the platform's maths library.
pub fn platform_maths(x: f64, y: f32) -> (f64, f32) {
    (x.sin() + x.powf(0.5) + libm::pow(x, 0.5), y.powi(2))
}
