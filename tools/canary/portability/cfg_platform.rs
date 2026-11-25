// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: file access hidden by a platform condition that no lint run in the check list
// compiles (the lint runs are on Linux). Clippy on Linux passes on this file; the
// portability script must not.
//
// check: script
// expect: uses `windows`, which is not in ALLOWED_CFG

//! Portability canary probe (never part of a build of the real crate).

/// Reads a whole file, on Windows only.
#[cfg(windows)]
pub fn read_all() -> std::io::Result<Vec<u8>> {
    std::fs::read("graph.eg")
}
