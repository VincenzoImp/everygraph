// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: the file system reached through the querying methods of `std::path::Path`,
// without naming `std::fs`.
//
// check: clippy
// expect: use of a disallowed method `std::path::Path::exists`
// expect: use of a disallowed method `std::path::Path::metadata`
// expect: use of a disallowed method `std::path::Path::read_dir`

//! Portability canary probe (never part of a build of the real crate).

use std::path::Path;

/// Looks a path up in the file system three ways.
pub fn look_up(path: &Path) -> bool {
    path.exists() && path.metadata().is_ok() && path.read_dir().is_ok()
}
