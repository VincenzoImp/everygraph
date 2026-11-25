// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: clock reads, which panic on wasm32-unknown-unknown.
//
// check: clippy
// expect: use of a disallowed method `std::time::Instant::now`
// expect: use of a disallowed method `std::time::SystemTime::elapsed`

//! Portability canary probe (never part of a build of the real crate).

/// Reads the monotonic clock and, by another name, the wall clock.
pub fn read_clocks() -> bool {
    let started = std::time::Instant::now();
    std::time::UNIX_EPOCH.elapsed().is_ok() && started.elapsed().as_nanos() < u128::MAX
}
