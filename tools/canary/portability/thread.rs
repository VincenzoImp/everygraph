// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: an OS thread started outside the parallel facade.
//
// check: clippy
// expect: use of a disallowed method `std::thread::spawn`

//! Portability canary probe (never part of a build of the real crate).

/// Starts a thread and waits for it.
pub fn start_thread() -> bool {
    std::thread::spawn(|| 1 + 1).join().is_ok()
}
