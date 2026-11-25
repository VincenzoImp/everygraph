// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: file access hidden from every lint run by a condition that is false under
// clippy. Clippy passes on this file; the portability script must not.
//
// check: script
// expect: uses `clippy`, which is not in ALLOWED_CFG

//! Portability canary probe (never part of a build of the real crate).

/// Reads a whole file where clippy cannot see it.
#[cfg(not(clippy))]
pub fn read_all() -> std::io::Result<Vec<u8>> {
    std::fs::read("graph.eg")
}
