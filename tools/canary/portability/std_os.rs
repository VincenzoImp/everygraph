// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: an operating-system module. No clippy list names it; it does not exist on
// wasm32-unknown-unknown, so the wasm32 build must fail.
//
// check: wasm32
// expect: could not find `unix` in `os`

//! Portability canary probe (never part of a build of the real crate).

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

/// Builds an `OsStr` from raw bytes, as only Unix allows.
pub fn from_bytes(bytes: &[u8]) -> &OsStr {
    OsStr::from_bytes(bytes)
}
