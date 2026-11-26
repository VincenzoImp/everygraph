// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A fresh directory per test, under Cargo's target directory: a disk file system, so
//! that direct I/O is really used on Linux. Shared by `tests/conformance.rs` and
//! `tests/container.rs` (included with `#[path]`).

use std::fs;
use std::path::{Path, PathBuf};

/// An empty directory `name`, private to the calling test binary.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("everygraph-io")
        .join(env!("CARGO_CRATE_NAME"))
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
