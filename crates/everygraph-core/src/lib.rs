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
//!   `everygraph-io`. [`Segments`] and [`SegmentReader`] cut a source into the bounded,
//!   aligned chunks that readers and kernels loop over.
//! - [`par`]: the only place where work is split across threads. It runs on rayon with
//!   the `parallel` feature and on the calling thread without it, with equal results.
//! - Memory accounting: [`Budget`] (the accountant), [`Memory`] (the handle kernels
//!   receive), [`Reservation`], and the containers that own both a buffer and its
//!   reservation: [`SegVec`], an array in segments of at most
//!   [`MAX_ALLOCATION_BYTES`], and [`ScratchPool`], per-thread scratch. Every large
//!   allocation goes through them, and a request that does not fit the budget is
//!   refused before anything is allocated (DESIGN D2, D11). [`MemoryProbe`] is the
//!   interface through which the layers above read what the machine has.
//!
//! # The portability rule
//!
//! This crate does not open files, map memory, start threads, read the clock or the
//! environment, or call the operating system. No single tool can check that, so it is
//! checked in pieces. Each piece covers exactly what is written next to it:
//!
//! - **Named items of `std`.** `clippy.toml` in this crate lists the items that reach
//!   the file system, threads, the clock, the environment, the process, the network
//!   and the standard streams. They compile for `wasm32-unknown-unknown` and fail or
//!   panic there at run time. Using one is a clippy error, and the crate root forbids
//!   the lints, so an inner `allow` is an error too. The list is kept by hand: an item
//!   that is missing from it is not caught.
//! - **Conditional compilation.** Code that clippy does not compile is not linted, so
//!   `tools/check_core_portability.py` accepts only a fixed set of conditions (tests,
//!   the `parallel` feature, the pointer width), and each has its own lint run.
//! - **`std::os`.** It does not exist on `wasm32-unknown-unknown`; the build for that
//!   target, without default features, rejects it.
//! - **Dependencies.** Neither clippy nor the wasm32 build sees what a dependency does:
//!   a crate that walks directories builds for wasm32 without complaint. The same
//!   script compares the dependency tree with an allow-list.
//! - **`unsafe`** is forbidden, so the crate cannot declare a foreign function or make
//!   a system call of its own.
//!
//! Not checked: blocking. `Mutex::lock`, `Condvar::wait`, `Barrier::wait` and a channel
//! `recv` compile everywhere and must not be reached on a browser's main thread.

// `forbid`, not `deny`: an inner `#[allow]` of any of these is a compile error (E0453).
// `tools/check_core_portability.py` checks that this line stays.
#![forbid(
    unsafe_code,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

mod byte_source;
mod ids;
mod memory;
pub mod par;
mod scratch;
mod seg_vec;
mod segment;

pub use byte_source::{AccessHint, ByteSource, MemorySource, ReadError};
pub use ids::{EdgeOffset, NodeId, NodeIdOverflow, OffsetOverflow};
pub use memory::{
    Budget, InsufficientBudget, Memory, MemoryAccountant, MemoryError, MemoryKind, MemoryProbe,
    MemoryReport, PurposeUsage, Reservation, SystemMemory,
};
pub use scratch::ScratchPool;
pub use seg_vec::{MAX_ALLOCATION_BYTES, SegVec};
pub use segment::{AlignedBuf, DEFAULT_SEGMENT_SIZE, SEGMENT_ALIGN, SegmentReader, Segments};

// Every conversion in this crate relies on these two facts: a `u32` always fits in a
// `usize`, and a `usize` always fits in a `u64`. A target where either is false must
// fail to compile rather than truncate.
const _: () = assert!(usize::BITS >= u32::BITS && usize::BITS <= u64::BITS);

/// Widens a length or index to the 64-bit type used for sizes that may be stored or
/// may describe something larger than memory.
pub(crate) const fn widen(value: usize) -> u64 {
    value as u64
}
