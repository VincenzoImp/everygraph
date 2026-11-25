// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`ScratchPool`]: per-thread scratch buffers, reserved for the scope that uses them.

use std::fmt;
use std::sync::{Mutex, TryLockError};

use crate::memory::{Memory, MemoryError, MemoryKind, Reservation};
use crate::{MAX_ALLOCATION_BYTES, widen};

/// Scratch buffers for the chunks of a parallel loop: one buffer per chunk that can run
/// at once, all reserved when the pool is made and released when it is dropped.
///
/// Per-thread scratch is the memory a lab accountant missed most easily: a hash table
/// or a dense array per worker, allocated inside the loop, multiplied by a thread count
/// nobody wrote down. A pool makes that product a reservation the plan can check
/// before work starts: `slots` is the number of chunks that can run at the same time,
/// which is the thread count of the plan (the core has no way to ask for it, on
/// purpose: see [`par`](crate::par)).
///
/// [`with`](Self::with) lends a free buffer to a chunk. It never waits: when every
/// buffer is in use (more chunks running at once than `slots`), the chunk gets a
/// temporary buffer, reserved for the duration of the call, and the call can then be
/// refused by the budget.
///
/// **Contract.** Every buffer is lent with all its elements equal to the pool's `fill`
/// value, and must be returned that way — the usual idiom is a dense array plus a list
/// of the positions touched, reset before returning. Which buffer a chunk gets depends
/// on timing, so a buffer returned dirty would make results depend on the schedule
/// (DESIGN D14).
pub struct ScratchPool<T> {
    slots: Vec<Mutex<Vec<T>>>,
    len: usize,
    fill: T,
    reservation: Reservation,
}

impl<T: Clone + Send> ScratchPool<T> {
    /// A pool of `slots` buffers of `len` elements, each filled with `fill`. The whole
    /// size is reserved before the first buffer is allocated.
    ///
    /// # Errors
    ///
    /// [`MemoryError::Budget`] if `slots × len` elements do not fit, before anything is
    /// allocated; [`MemoryError::AllocationFailed`] if one buffer would exceed
    /// [`MAX_ALLOCATION_BYTES`] or the allocator refused.
    pub fn try_new(
        memory: &Memory,
        purpose: &'static str,
        slots: usize,
        len: usize,
        fill: T,
    ) -> Result<Self, MemoryError> {
        let per_slot = buffer_bytes::<T>(purpose, len)?;
        let total = per_slot
            .checked_mul(widen(slots))
            .ok_or(MemoryError::AllocationFailed {
                purpose,
                bytes: u64::MAX,
            })?;
        let reservation = memory.reserve(purpose, MemoryKind::Anonymous, total)?;
        let mut pool = Vec::new();
        pool.try_reserve_exact(slots)
            .map_err(|_| MemoryError::AllocationFailed {
                purpose,
                bytes: u64::MAX,
            })?;
        for _ in 0..slots {
            pool.push(Mutex::new(filled(purpose, len, &fill)?));
        }
        Ok(Self {
            slots: pool,
            len,
            fill,
            reservation,
        })
    }

    /// Calls `f` with a buffer of [`len`](Self::len) elements, all equal to the fill
    /// value, which `f` must restore before it returns (type documentation).
    ///
    /// # Errors
    ///
    /// Only when every pooled buffer is in use: the temporary buffer then given to `f`
    /// is reserved first, and that can be refused ([`MemoryError::Budget`]) or fail
    /// ([`MemoryError::AllocationFailed`]). `f` is not called in that case.
    pub fn with<R>(&self, f: impl FnOnce(&mut [T]) -> R) -> Result<R, MemoryError> {
        for slot in &self.slots {
            match slot.try_lock() {
                Ok(mut buffer) => return Ok(f(&mut buffer)),
                // A buffer whose user panicked may be dirty: it is not lent again.
                Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => {}
            }
        }
        let purpose = self.reservation.purpose();
        let _temporary = self.reservation.memory().reserve(
            purpose,
            MemoryKind::Anonymous,
            buffer_bytes::<T>(purpose, self.len)?,
        )?;
        let mut buffer = filled(purpose, self.len, &self.fill)?;
        Ok(f(&mut buffer))
    }

    /// Elements per buffer.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffers are empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of pooled buffers.
    #[must_use]
    pub fn slots(&self) -> usize {
        self.slots.len()
    }

    /// Bytes reserved for the pooled buffers.
    #[must_use]
    pub const fn reserved_bytes(&self) -> u64 {
        self.reservation.bytes()
    }
}

/// Bytes of one buffer of `len` elements, refused if it exceeds a single allocation.
fn buffer_bytes<T>(purpose: &'static str, len: usize) -> Result<u64, MemoryError> {
    widen(len)
        .checked_mul(widen(size_of::<T>()))
        .filter(|&bytes| bytes <= MAX_ALLOCATION_BYTES)
        .ok_or(MemoryError::AllocationFailed {
            purpose,
            bytes: widen(len).saturating_mul(widen(size_of::<T>())),
        })
}

/// A vector of exactly `len` copies of `fill`, allocated fallibly. The caller has
/// reserved it.
fn filled<T: Clone>(purpose: &'static str, len: usize, fill: &T) -> Result<Vec<T>, MemoryError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(len)
        .map_err(|_| MemoryError::AllocationFailed {
            purpose,
            bytes: widen(len).saturating_mul(widen(size_of::<T>())),
        })?;
    buffer.resize(len, fill.clone());
    Ok(buffer)
}

impl<T> fmt::Debug for ScratchPool<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScratchPool")
            .field("purpose", &self.reservation.purpose())
            .field("slots", &self.slots.len())
            .field("len", &self.len)
            .field("reserved_bytes", &self.reservation.bytes())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::memory::{Budget, InsufficientBudget};
    use crate::par;

    #[test]
    fn the_pool_is_reserved_up_front_and_released_on_drop() {
        let budget = Arc::new(Budget::new(1000));
        let memory = Memory::from(Arc::clone(&budget));
        let pool = ScratchPool::try_new(&memory, "local moving", 4, 25, 0_u64).unwrap();
        assert_eq!((pool.reserved_bytes(), budget.used()), (800, 800));
        assert_eq!(
            ScratchPool::try_new(&memory, "refinement", 2, 13, 0_u64).unwrap_err(),
            MemoryError::Budget(InsufficientBudget::new("refinement", 208, 200))
        );
        drop(pool);
        assert_eq!(budget.used(), 0);
    }

    /// Chunks running in parallel each get a clean buffer, and whatever the schedule
    /// the answer is the same.
    #[test]
    fn parallel_chunks_share_the_pool() {
        let memory = Memory::unlimited();
        let pool = ScratchPool::try_new(&memory, "counts", 2, 16, 0_u32).unwrap();
        let total = AtomicU64::new(0);
        par::for_each_range(1000, NonZeroUsize::new(10).unwrap(), |range| {
            pool.with(|counts| {
                assert!(counts.iter().all(|&c| c == 0), "lent clean");
                for i in range.clone() {
                    counts[i % 16] += 1;
                }
                let sum: u32 = counts.iter().sum();
                total.fetch_add(u64::from(sum), Ordering::Relaxed);
                counts.fill(0);
            })
            .unwrap();
        });
        assert_eq!(total.load(Ordering::Relaxed), 1000);
    }

    /// With every buffer in use, the next chunk gets a temporary one, reserved while
    /// it is used — or refused by the budget, without calling the chunk.
    #[test]
    fn a_chunk_beyond_the_slots_gets_a_reserved_temporary_buffer() {
        let budget = Arc::new(Budget::new(100));
        let memory = Memory::from(Arc::clone(&budget));
        let pool = ScratchPool::try_new(&memory, "table", 1, 10, 0_u8).unwrap();
        pool.with(|_| {
            let inner = pool.with(|buffer| {
                assert_eq!(buffer.len(), 10);
                budget.used()
            });
            assert_eq!(inner, Ok(20), "pooled + temporary");
        })
        .unwrap();
        assert_eq!(budget.used(), 10);

        let tight = Arc::new(Budget::new(15));
        let memory = Memory::from(Arc::clone(&tight));
        let pool = ScratchPool::try_new(&memory, "table", 1, 10, 0_u8).unwrap();
        pool.with(|_| {
            let mut called = false;
            let refused = pool.with(|_| called = true).unwrap_err();
            assert_eq!(
                refused,
                MemoryError::Budget(InsufficientBudget::new("table", 10, 5))
            );
            assert!(!called);
        })
        .unwrap();
    }

    #[test]
    fn a_buffer_above_the_allocation_limit_is_refused() {
        let memory = Memory::unlimited();
        let too_big = usize::try_from(MAX_ALLOCATION_BYTES / 4 + 1).unwrap();
        assert!(matches!(
            ScratchPool::try_new(&memory, "dense", 1, too_big, 0_u32),
            Err(MemoryError::AllocationFailed {
                purpose: "dense",
                ..
            })
        ));
    }
}
