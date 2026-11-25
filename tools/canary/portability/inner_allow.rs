// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: the lint switched off by an inner `allow`. The crate root forbids it, so this
// is error E0453 under clippy (plain `cargo build` accepts it).
//
// check: clippy
// expect: error[E0453]: allow(clippy::disallowed_methods) incompatible with previous forbid

//! Portability canary probe (never part of a build of the real crate).

/// Reads a whole file, with the lint allowed.
#[allow(clippy::disallowed_methods)]
pub fn read_all() -> std::io::Result<Vec<u8>> {
    std::fs::read("graph.eg")
}
