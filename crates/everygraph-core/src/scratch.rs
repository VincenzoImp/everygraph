// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`ScratchPool`]: per-thread scratch buffers, reserved for the scope that uses them.

use std::alloc::Allocator;
use std::cell::Cell;
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
/// before work starts.
///
/// [`with`](Self::with) lends a free buffer to a chunk. It never waits: when every
/// buffer is in use, the chunk gets a temporary buffer, reserved for the duration of
/// the call, and the call can then be refused by the budget.
///
/// **Planner rule.** A refusal that depends on timing must not happen, so:
/// - `slots` is at least the number of threads of the pool that runs the loop — the
///   plan's thread count, which the caller passes in (the core cannot ask for it, on
///   purpose: see [`par`](crate::par)). A thread pool larger than the plan
///   (`RAYON_NUM_THREADS` above it, say) makes more chunks run at once than there are
///   slots.
/// - `f` does not call [`par`](crate::par): a worker waiting inside a nested parallel
///   call may start another chunk of the outer loop while it still holds its buffer.
///   Debug builds panic when a thread re-enters `with` of the same pool.
///
/// **Contract.** Every buffer is lent with all its elements equal to the pool's `fill`
/// value, and must be returned that way — the usual idiom is a dense array plus a list
/// of the positions touched, reset before returning. Which buffer a chunk gets depends
/// on timing, so a buffer returned dirty would make results depend on the schedule
/// (DESIGN D14). Debug builds check it on every return (so `fill` must equal itself: not
/// a NaN). A buffer whose user panicked is never lent again.
///
/// Elements are `Copy`, so that a buffer holds no heap memory outside the budget. The
/// buffers, temporary ones included, are allocated from the accountant's
/// [`Pages`](crate::Pages).
pub struct ScratchPool<T: Copy> {
    slots: Vec<Mutex<Buffer<T>>>,
    len: usize,
    fill: T,
    reservation: Reservation,
}

type Buffer<T> = Vec<T, &'static (dyn Allocator + Sync)>;

thread_local! {
    /// The address of the pool whose buffer this thread holds, or 0.
    static LENT: Cell<usize> = const { Cell::new(0) };
}

/// Restores [`LENT`] when a call to `with` ends, also by a panic.
struct Lent(usize);

impl Drop for Lent {
    fn drop(&mut self) {
        LENT.with(|lent| lent.set(self.0));
    }
}

impl<T: Copy + PartialEq + Send> ScratchPool<T> {
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
        let copy = fill;
        debug_assert!(
            copy == fill,
            "the fill value of {purpose} is not equal to itself"
        );
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
                bytes: widen(slots).saturating_mul(widen(size_of::<Mutex<Buffer<T>>>())),
            })?;
        let allocator = memory.pages().allocator();
        for _ in 0..slots {
            pool.push(Mutex::new(filled(allocator, purpose, len, fill)?));
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
    ///
    /// # Panics
    ///
    /// In debug builds, if `f` returns the buffer with an element not equal to the fill
    /// value, or if the thread already holds a buffer of this pool (planner rule).
    pub fn with<R>(&self, f: impl FnOnce(&mut [T]) -> R) -> Result<R, MemoryError> {
        let me = std::ptr::from_ref(self).addr();
        let _restore = Lent(LENT.with(|lent| lent.replace(me)));
        debug_assert_ne!(
            _restore.0,
            me,
            "ScratchPool::with of {} re-entered on one thread: no `par` inside `with`",
            self.reservation.purpose()
        );
        let lend = |buffer: &mut [T]| {
            let result = f(buffer);
            debug_assert!(
                buffer.iter().all(|x| *x == self.fill),
                "a buffer of {} was returned with elements not equal to the fill value",
                self.reservation.purpose()
            );
            result
        };
        for slot in &self.slots {
            match slot.try_lock() {
                Ok(mut buffer) => return Ok(lend(&mut buffer)),
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
        let allocator = self.reservation.memory().pages().allocator();
        let mut buffer = filled(allocator, purpose, self.len, self.fill)?;
        Ok(lend(&mut buffer))
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

/// A vector of exactly `len` copies of `fill`, allocated fallibly from `allocator`. The
/// caller has reserved it.
fn filled<T: Copy>(
    allocator: &'static (dyn Allocator + Sync),
    purpose: &'static str,
    len: usize,
    fill: T,
) -> Result<Buffer<T>, MemoryError> {
    let mut buffer = Vec::new_in(allocator);
    buffer
        .try_reserve_exact(len)
        .map_err(|_| MemoryError::AllocationFailed {
            purpose,
            bytes: widen(len).saturating_mul(widen(size_of::<T>())),
        })?;
    buffer.resize(len, fill);
    Ok(buffer)
}

impl<T: Copy> fmt::Debug for ScratchPool<T> {
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
    use std::panic::AssertUnwindSafe;
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
        {
            // The only slot is in use by another chunk.
            let _busy = pool.slots[0].lock().unwrap();
            let inner = pool.with(|buffer| {
                assert_eq!(buffer.len(), 10);
                budget.used()
            });
            assert_eq!(inner, Ok(20), "pooled + temporary");
        }
        assert_eq!(budget.used(), 10);

        let tight = Arc::new(Budget::new(15));
        let memory = Memory::from(Arc::clone(&tight));
        let pool = ScratchPool::try_new(&memory, "table", 1, 10, 0_u8).unwrap();
        let _busy = pool.slots[0].lock().unwrap();
        let mut called = false;
        let refused = pool.with(|_| called = true).unwrap_err();
        assert_eq!(
            refused,
            MemoryError::Budget(InsufficientBudget::new("table", 10, 5))
        );
        assert!(!called);
    }

    /// A buffer whose user panicked is not lent again: the next chunk gets a clean
    /// (temporary) one.
    #[test]
    fn a_poisoned_buffer_is_not_lent_again() {
        let budget = Arc::new(Budget::new(100));
        let memory = Memory::from(Arc::clone(&budget));
        let pool = ScratchPool::try_new(&memory, "table", 1, 4, 0_u8).unwrap();
        let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| {
            pool.with(|buffer| {
                buffer[0] = 9;
                panic!("chunk failed");
            })
        }));
        assert!(panicked.is_err());
        let clean = pool
            .with(|buffer| (buffer.iter().all(|&b| b == 0), budget.used()))
            .unwrap();
        assert_eq!(clean, (true, 8), "a clean temporary buffer, reserved");
        assert_eq!(budget.used(), 4);
    }

    /// Whether `debug_assert!` is compiled in, without a `cfg` the portability check
    /// would have to allow.
    fn debug_build() -> bool {
        let mut debug = false;
        debug_assert!({
            debug = true;
            true
        });
        debug
    }

    #[test]
    fn debug_builds_catch_a_dirty_return_and_a_nested_use() {
        let memory = Memory::unlimited();
        let pool = ScratchPool::try_new(&memory, "table", 2, 4, 0_u32).unwrap();
        let dirty =
            std::panic::catch_unwind(AssertUnwindSafe(|| pool.with(|buffer| buffer[1] = 5)));
        assert_eq!(dirty.is_err(), debug_build());

        let pool = ScratchPool::try_new(&memory, "table", 2, 4, 0_u32).unwrap();
        let nested =
            std::panic::catch_unwind(AssertUnwindSafe(|| pool.with(|_| pool.with(|_| ()))));
        assert_eq!(nested.is_err(), debug_build());
        // Two different pools may be nested, and the thread's record is restored.
        let other = ScratchPool::try_new(&memory, "other", 1, 4, 0_u32).unwrap();
        assert!(pool.with(|_| other.with(|_| ()).is_ok()).unwrap());
        assert!(pool.with(|_| ()).is_ok());
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
