// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`ReadPool`]: threads that make blocking positional reads of one source, the
//! read-ahead of a [`BlockCache`](everygraph_core::BlockCache).
//!
//! A pool of `pread` threads, not `io_uring`: in m2-35 the pool reached 0.99–1.03 of the
//! best `io_uring` variant's random-read rate in every cell, and at least 0.94
//! (`lab/m2-35-random-read-probe/README.md`, exploratory). On that drive 64 threads
//! saturate it with 16 and 64 KiB reads; 4 KiB reads still gained up to 256.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use everygraph_core::{
    AlignedBuf, ByteSource, Fetch, Memory, MemoryError, MemoryKind, ReadAhead, SEGMENT_ALIGN,
};

/// How a [`ReadPool`] is sized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolSettings {
    /// Reader threads, each with one read in flight: the queue depth. 64 by default
    /// (m2-35).
    pub threads: usize,
    /// Bytes of each thread's buffer: the longest read. 64 KiB by default (m2-35).
    pub max_read: usize,
    /// Reads queued at most; more are refused, and their bytes are read when needed.
    pub queue: usize,
}

impl Default for PoolSettings {
    fn default() -> Self {
        Self {
            threads: 64,
            max_read: 64 << 10,
            queue: 1 << 14,
        }
    }
}

struct Queue {
    fetches: VecDeque<Fetch>,
    closed: bool,
    running: usize,
}

/// Reader threads over one source, for [`BlockCache::with_read_ahead`]. Made and
/// stopped by [`ReadPool::run`].
///
/// [`BlockCache::with_read_ahead`]: everygraph_core::BlockCache::with_read_ahead
pub struct ReadPool<'s> {
    source: &'s (dyn ByteSource + Sync),
    queue: Mutex<Queue>,
    work: Condvar,
    settings: PoolSettings,
}

impl std::fmt::Debug for ReadPool<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadPool")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

fn lock(queue: &Mutex<Queue>) -> MutexGuard<'_, Queue> {
    queue.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Closes the pool when `run`'s closure returns or unwinds, so its threads end.
struct Close<'p, 's>(&'p ReadPool<'s>);

impl Drop for Close<'_, '_> {
    fn drop(&mut self) {
        let dropped = {
            let mut q = lock(&self.0.queue);
            q.closed = true;
            std::mem::take(&mut q.fetches)
        };
        // Outside the lock: each gives its frames back.
        drop(dropped);
        self.0.work.notify_all();
    }
}

impl<'s> ReadPool<'s> {
    /// Starts the threads over `source`, calls `f` with the pool, and stops them. The
    /// threads' buffers and the queue are reserved in `memory` first. A thread that the
    /// system cannot start is left out; with none, every read is made when needed.
    ///
    /// # Errors
    ///
    /// [`MemoryError`] if the buffers or the queue do not fit; nothing is started then.
    pub fn run<R>(
        source: &'s (dyn ByteSource + Sync),
        memory: &Memory,
        settings: PoolSettings,
        f: impl FnOnce(&ReadPool<'s>) -> R,
    ) -> Result<R, MemoryError> {
        let threads = settings.threads.max(1);
        let buffer = (settings.max_read + SEGMENT_ALIGN) as u64;
        let queue = (settings.queue as u64).saturating_mul(size_of::<Fetch>() as u64);
        let _held = memory.reserve(
            "read pool buffers",
            MemoryKind::Anonymous,
            (threads as u64)
                .saturating_mul(buffer)
                .saturating_add(queue),
        )?;
        let mut fetches = VecDeque::new();
        fetches
            .try_reserve_exact(settings.queue)
            .map_err(|_| MemoryError::AllocationFailed {
                purpose: "read pool buffers",
                bytes: queue,
            })?;
        let buffers = (0..threads)
            .map(|_| {
                AlignedBuf::try_zeroed(settings.max_read).map_err(|_| {
                    MemoryError::AllocationFailed {
                        purpose: "read pool buffers",
                        bytes: buffer,
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let pool = ReadPool {
            source,
            queue: Mutex::new(Queue {
                fetches,
                closed: false,
                running: 0,
            }),
            work: Condvar::new(),
            settings,
        };
        Ok(std::thread::scope(|scope| {
            let close = Close(&pool);
            for buf in buffers {
                let pool = &pool;
                let started = std::thread::Builder::new()
                    .name("everygraph-read".into())
                    .spawn_scoped(scope, move || pool.serve(buf));
                if started.is_ok() {
                    lock(&pool.queue).running += 1;
                }
            }
            let result = f(&pool);
            drop(close);
            result
        }))
    }

    /// A thread's loop: runs queued fetches until the pool is closed.
    fn serve(&self, mut buf: AlignedBuf) {
        loop {
            let fetch = {
                let mut q = lock(&self.queue);
                loop {
                    if q.closed {
                        return;
                    }
                    if let Some(fetch) = q.fetches.pop_front() {
                        break fetch;
                    }
                    q = self.work.wait(q).unwrap_or_else(PoisonError::into_inner);
                }
            };
            // A failed read gives its frames back; the reader that needs them reads them
            // itself and gets the error.
            let _ = fetch.run(self.source, buf.as_mut_slice());
        }
    }
}

impl ReadAhead for ReadPool<'_> {
    fn source(&self) -> &(dyn ByteSource + Sync) {
        self.source
    }

    fn submit(&self, fetch: Fetch) -> Result<(), Fetch> {
        if fetch.len() > self.settings.max_read {
            return Err(fetch);
        }
        let mut q = lock(&self.queue);
        if q.closed || q.running == 0 || q.fetches.len() >= self.settings.queue {
            return Err(fetch);
        }
        q.fetches.push_back(fetch);
        drop(q);
        self.work.notify_one();
        Ok(())
    }

    fn max_read(&self) -> usize {
        self.settings.max_read
    }
}
