// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: a dependency that does the file-system access itself. Clippy, the wasm32
// build and `cargo deny` all accept it; only the dependency allow-list stops it.
//
// dependency: walkdir = "2"
// check: script
// expect: dependency `walkdir` of everygraph-core is not in ALLOWED_DEPENDENCIES

//! Portability canary probe (never part of a build of the real crate).

/// Counts the entries under the current directory.
pub fn count_entries() -> usize {
    walkdir::WalkDir::new(".").into_iter().count()
}
