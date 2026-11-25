// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: the file system reached through `std::fs` directly.
//
// check: clippy
// expect: use of a disallowed method `std::fs::read`
// expect: use of a disallowed type `std::fs::File`

//! Portability canary probe (never part of a build of the real crate).

/// Reads a whole file.
pub fn read_all() -> std::io::Result<Vec<u8>> {
    std::fs::read("graph.eg")
}

/// Opens a file.
pub fn open() -> std::io::Result<std::fs::File> {
    std::fs::File::open("graph.eg")
}
