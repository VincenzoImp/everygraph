// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Every path of `everygraph_core::SegVec` and `ScratchPool` where the budget says yes
//! and the allocator says no: the error is `MemoryError::AllocationFailed`, and the
//! budget is left holding exactly what is still allocated.
//!
//! It needs a global allocator that can refuse, which needs `unsafe`. The test lives
//! here because `everygraph-core` forbids `unsafe` in all its crate roots, tests
//! included (`tools/check_core_portability.py`), and this crate is where the
//! operating system — and its `unsafe` — is allowed.

use std::sync::{Arc, Barrier};

use everygraph_core::{Budget, Memory, MemoryError, ScratchPool, SegVec};

mod refusing {
    //! A global allocator that passes everything to `System`, except that the test
    //! thread can make it refuse large requests. Safe code cannot replace the global
    //! allocator, hence the module's `unsafe` (CONTRIBUTING.md, "`unsafe`").
    #![allow(unsafe_code)]

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        /// (size from which requests count, how many of those are still granted).
        static RULE: Cell<(usize, usize)> = const { Cell::new((usize::MAX, 0)) };
    }

    /// Whether a request of `size` bytes from this thread is refused.
    fn refused(size: usize) -> bool {
        RULE.try_with(|rule| {
            let (from, granted) = rule.get();
            if size < from {
                false
            } else if granted > 0 {
                rule.set((from, granted - 1));
                false
            } else {
                true
            }
        })
        .unwrap_or(false)
    }

    /// Runs `f` with requests of at least `from` bytes refused on this thread, after
    /// the first `granted` of them.
    pub fn refusing<R>(from: usize, granted: usize, f: impl FnOnce() -> R) -> R {
        RULE.with(|rule| rule.set((from, granted)));
        let result = f();
        RULE.with(|rule| rule.set((usize::MAX, 0)));
        result
    }

    pub struct Refusing;

    // SAFETY: every block of memory comes from `System` and goes back to it with the
    // layout it was allocated with; a refusal returns null, which `GlobalAlloc` allows
    // (and for `realloc` leaves the old block untouched, as required).
    unsafe impl GlobalAlloc for Refusing {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if refused(layout.size()) {
                return std::ptr::null_mut();
            }
            // SAFETY: the caller's guarantees for `alloc` (a non-zero size) are passed on.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: `ptr` was allocated by `System` with `layout` (every allocation
            // and reallocation here is `System`'s).
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            if refused(new_size) {
                return std::ptr::null_mut();
            }
            // SAFETY: `ptr` and `layout` are `System`'s, and the caller's guarantees for
            // `new_size` are passed on.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: Refusing = Refusing;
}

use refusing::refusing;

fn budget() -> (Arc<Budget>, Memory) {
    let budget = Arc::new(Budget::new(1 << 20));
    let memory = Memory::from(Arc::clone(&budget));
    (budget, memory)
}

fn failed(purpose: &'static str, bytes: u64) -> MemoryError {
    MemoryError::AllocationFailed { purpose, bytes }
}

#[test]
fn a_refused_new_vector_holds_nothing() {
    let (budget, memory) = budget();
    let result = refusing(1000, 0, || {
        SegVec::try_with_len(&memory, "ranks", 4096, 0_u8)
    });
    assert_eq!(result.unwrap_err(), failed("ranks", 4096));
    assert_eq!(budget.used(), 0);
}

/// The last segment is reallocated and the allocator refuses: the old buffer, its
/// elements and its reservation stay, and the transient reservation is given back.
#[test]
fn a_refused_reallocation_keeps_the_old_segment() {
    let (budget, memory) = budget();
    let mut vec = SegVec::<u8>::with_segment_bits(&memory, "bytes", 12);
    vec.try_extend_from_slice(&[3; 100]).unwrap();
    vec.shrink_to_fit();
    assert_eq!(budget.used(), 100);
    let result = refusing(500, 0, || vec.try_reserve(900));
    assert_eq!(result.unwrap_err(), failed("bytes", 1000));
    assert_eq!((vec.capacity(), budget.used()), (100, 100));
    assert!(vec.iter().all(|&b| b == 3));
}

/// New segments: the first is granted, the second refused. The vector keeps the
/// first, and the budget holds exactly it.
#[test]
fn a_refused_segment_keeps_those_already_allocated() {
    let (budget, memory) = budget();
    let mut vec = SegVec::<u8>::with_segment_bits(&memory, "bytes", 8);
    let result = refusing(256, 1, || vec.try_reserve(600));
    assert_eq!(result.unwrap_err(), failed("bytes", 256));
    assert_eq!((vec.capacity(), budget.used()), (256, 256));
    assert_eq!(vec.reserved_bytes(), 256);
    vec.try_reserve(600).unwrap();
    assert_eq!(budget.used(), 600);
}

#[test]
fn a_refused_clone_holds_nothing() {
    let (budget, memory) = budget();
    let vec = SegVec::try_with_len(&memory, "ids", 2000, 1_u32).unwrap();
    let result = refusing(4000, 0, || vec.try_clone());
    assert_eq!(result.unwrap_err(), failed("ids", 8000));
    assert_eq!(budget.used(), 8000);
}

#[test]
fn a_refused_pool_holds_nothing() {
    let (budget, memory) = budget();
    // The second buffer is refused.
    let result = refusing(4096, 1, || {
        ScratchPool::try_new(&memory, "tables", 2, 4096, 0_u8)
    });
    assert_eq!(result.unwrap_err(), failed("tables", 4096));
    assert_eq!(budget.used(), 0);

    // The vector of slots itself is refused (1000 slots of one byte each).
    let result = refusing(8000, 0, || {
        ScratchPool::try_new(&memory, "flags", 1000, 1, 0_u8)
    });
    assert!(matches!(
        result,
        Err(MemoryError::AllocationFailed {
            purpose: "flags",
            ..
        })
    ));
    assert_eq!(budget.used(), 0);
}

/// Every slot is in use by another thread; the temporary buffer is refused by the
/// allocator, `f` is not called, and only the pool stays reserved.
#[test]
fn a_refused_temporary_buffer_holds_nothing() {
    let (budget, memory) = budget();
    let pool = ScratchPool::try_new(&memory, "tables", 1, 4096, 0_u8).unwrap();
    let holding = Barrier::new(2);
    let release = Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            pool.with(|_| {
                holding.wait();
                release.wait();
            })
            .unwrap();
        });
        holding.wait();
        let mut called = false;
        let result = refusing(1000, 0, || pool.with(|_| called = true));
        assert_eq!(result.unwrap_err(), failed("tables", 4096));
        assert!(!called);
        assert_eq!(budget.used(), 4096);
        release.wait();
    });
    assert_eq!(budget.used(), 4096);
}
