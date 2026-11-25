// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native I/O for Everygraph: the crate where the file system and the operating system
//! are allowed. Everything above reads through [`ByteSource`] and stays portable
//! (DESIGN D2); this crate supplies the backends a native build plugs in.
//!
//! **Sources** (all implement [`ByteSource`] and are `Send + Sync`):
//!
//! | Backend | For | Zero-copy | Advice |
//! |---|---|---|---|
//! | [`MemorySource`] (from the core) | bytes already in memory | yes | ignored |
//! | [`MmapSource`] | the in-RAM mode: a mapped file, warm or under pressure (D11) | yes | `madvise` |
//! | [`FileSource::open`] | positional reads through the page cache | no | `posix_fadvise` |
//! | [`FileSource::open_direct`] | a large input read once (`O_DIRECT`, `F_NOCACHE` on macOS) | no | ignored |
//!
//! Readers and kernels loop over a source in bounded chunks with
//! [`Segments`](everygraph_core::Segments) and
//! [`SegmentReader`](everygraph_core::SegmentReader), one reader per worker, which
//! zero-copy backends serve in place and direct backends fill without a bounce copy.
//!
//! **Sinks:** [`AppendSink`] (buffered, front to back), [`BlockWriter`] (a preallocated
//! file filled in aligned blocks by many threads), [`SpillLogs`] (sharded temporary
//! logs read back as [`FileSource`]s). See the [`sink`] module for which lab result
//! each answers.
//!
//! **Later backends** need no change here or in the trait. An HTTP-range source
//! implements `len` and `read_at` (one range request each, or served from a block
//! cache) and may take [`AccessHint::WillNeed`] as the signal to prefetch; an
//! `io_uring` source does the same with a submission queue. Both are planned, not built.
//!
//! **Memory:** [`ProcMemoryProbe`], the Linux implementation of
//! `everygraph_core::MemoryProbe`: physical memory and the limits of the process's
//! control group. [`limit_allocator_retention`], a stopgap for glibc until the
//! allocator policy (issue `m2-19-allocator-policy`).
//!
//! Every `unsafe` block of the crate is in one private module (`sys`): the memory map
//! and five calls into the C library. The map is safe to offer under the invariant
//! stated on [`MmapSource`], which the sinks enforce within the process.

pub mod sink;

mod claim;
mod file;
mod memory_probe;
mod mmap;
mod sys;

use std::io;

use everygraph_core::ReadError;
pub use everygraph_core::{AccessHint, ByteSource, MemorySource};

pub use file::FileSource;
pub use memory_probe::ProcMemoryProbe;
pub use mmap::MmapSource;
pub use sink::{AppendSink, BlockWriter, SpillLogs};

/// What [`limit_allocator_retention`] fixes glibc's mmap and trim thresholds at: 128 KiB,
/// glibc's own default.
pub const MALLOC_THRESHOLD_BYTES: i32 = 128 << 10;

/// Stops glibc's `malloc` from keeping freed memory that the memory budget no longer
/// counts. Call it once, at the start of a program, before it spawns threads; a no-op
/// that returns false on any other C library or target.
///
/// glibc raises its mmap threshold, by default, to the size of each large block freed,
/// up to 32 MiB: from then on, blocks below it come from per-thread arenas, and a freed
/// one stays resident there instead of going back to the system. With 16 threads, the
/// identifier dictionary's 500 M-key build under a 4 GiB budget then held up to 1 GiB
/// of anonymous memory above its ledger and was killed at `MemoryMax=5G` (PR #158's
/// benchmark). Fixing the mmap and trim thresholds (`mallopt`, which also turns that
/// growth off) keeps every block of [`MALLOC_THRESHOLD_BYTES`] or more in its own map,
/// returned when freed. The allocator policy proper — large buffers that never go
/// through `malloc`, a cgroup monitor — is issue `m2-19-allocator-policy`.
pub fn limit_allocator_retention() -> bool {
    sys::pin_malloc_thresholds(MALLOC_THRESHOLD_BYTES)
}

/// The error for a read of `len` bytes at `offset` outside a source of `source_len`.
fn check_bounds(offset: u64, len: usize, source_len: u64) -> Result<(), ReadError> {
    let len = len as u64;
    match offset.checked_add(len) {
        Some(end) if end <= source_len => Ok(()),
        _ => Err(ReadError::OutOfBounds {
            offset,
            len,
            source_len,
        }),
    }
}

fn backend_error(error: io::Error) -> ReadError {
    ReadError::Backend(Box::new(error))
}
