// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Probe: a DNS lookup, which needs no socket type to be named.
//
// check: clippy
// expect: use of a disallowed method `std::net::ToSocketAddrs::to_socket_addrs`

//! Portability canary probe (never part of a build of the real crate).

use std::net::ToSocketAddrs;

/// Resolves a host name.
pub fn resolve() -> bool {
    "example.org:80".to_socket_addrs().is_ok()
}
