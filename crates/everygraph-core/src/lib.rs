// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Portable foundations of Everygraph.
//!
//! Everything above this crate — formats, ingest, kernels, communities, the map — is
//! written against the few types defined here, so that the same code runs on a
//! workstation, out of core, and in a browser tab (DESIGN D2). What lives here:
//!
//! - [`NodeId`] and [`EdgeOffset`]: 32-bit node ids, 64-bit offsets, and the checked
//!   conversions between them and `usize` (DESIGN D3).
//! - [`ByteSource`]: positional, re-readable access to bytes, read in bounded chunks.
//!   [`MemorySource`] is the in-memory backend; files, mmap and HTTP ranges live in
//!   `everygraph-io`.
//! - [`par`]: the only place where work is split across threads. It runs on rayon with
//!   the `parallel` feature and on the calling thread without it, with equal results.
//! - [`MemoryAccountant`]: the interface through which large allocations are reserved
//!   against the user's memory budget. The implementation arrives with issue #31.
//!
//! # The portability rule
//!
//! This crate does not open files, map memory, start threads, read the clock or the
//! environment, or call the operating system. The rule is enforced three ways:
//!
//! - `clippy.toml` in this crate lists the `std` items that compile for
//!   `wasm32-unknown-unknown` but fail or panic there at run time (`std::fs`,
//!   `std::thread`, `std::time::Instant::now`, …); using one is a clippy error.
//! - The crate is built for `wasm32-unknown-unknown` without default features, which
//!   rejects anything under `std::os` and any dependency that needs an OS.
//! - `unsafe` is forbidden, so no system call can be made behind the back of the two
//!   checks above; `deny.toml` keeps memory-mapping crates out of the dependencies.

#![forbid(unsafe_code)]

mod byte_source;
mod ids;
mod memory;
pub mod par;

pub use byte_source::{ByteSource, MemorySource, ReadError};
pub use ids::{EdgeOffset, NodeId, NodeIdOverflow, OffsetOverflow};
pub use memory::{InsufficientBudget, MemoryAccountant};

// Every conversion in this crate relies on these two facts: a `u32` always fits in a
// `usize`, and a `usize` always fits in a `u64`. A target where either is false must
// fail to compile rather than truncate.
const _: () = assert!(usize::BITS >= u32::BITS && usize::BITS <= u64::BITS);

/// Widens a length or index to the 64-bit type used for sizes that may be stored or
/// may describe something larger than memory.
pub(crate) const fn widen(value: usize) -> u64 {
    value as u64
}
