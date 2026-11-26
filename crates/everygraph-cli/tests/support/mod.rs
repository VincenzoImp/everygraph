// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the tests of the command share: running it, a directory per test, and the
//! closed checks of its JSON documents.

#![allow(dead_code, reason = "each test file uses part of it")]

pub mod schema;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Runs the command in `dir` with standard input closed (S-4).
pub fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_everygraph"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .env_remove("NO_COLOR")
        .env("LANG", "C.UTF-8")
        .output()
        .unwrap()
}

/// A fresh directory for one test.
pub fn dir(group: &str, name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(group)
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Standard output as one JSON document, and nothing else (S-2).
pub fn document(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}{}", stdout(out), stderr(out)))
}
