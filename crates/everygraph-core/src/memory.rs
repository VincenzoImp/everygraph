// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The memory accountant interface.
//!
//! The promise "never run out of memory" (DESIGN § 2, D11) is kept by asking before
//! allocating: every large buffer is reserved against the user's budget first, and a
//! request that does not fit is refused with a typed error while nothing has been
//! allocated yet. The only out-of-memory kill in the lab came from O(n) state that
//! nobody had counted.
//!
//! This module defines only the interface, so that code written from now on can take an
//! accountant as a parameter. The accountant itself (budget, peak tracking, the policy
//! for file-backed memory, reservation guards) is issue #31.

use std::error::Error;
use std::fmt;

/// What a reservation will be backed by. The two kinds behave differently when memory
/// runs short, which is why every reservation states its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MemoryKind {
    /// Memory with no file behind it: heap buffers, kernel state, decode caches. The
    /// operating system can only keep it or kill the process, so this is what the
    /// budget limits.
    Anonymous,
    /// Pages of a file mapped into memory. The operating system can drop them and read
    /// them again, so they are reclaimable; an accountant records them but need not
    /// count them against the budget the way it counts anonymous memory.
    FileBacked,
}

/// Keeps count of the memory a computation has claimed out of a fixed budget.
///
/// Sizes are `u64` on every target: a plan is checked before anything is allocated, and
/// the amount a graph would need can exceed what a 32-bit `usize` can express — which
/// is exactly the request that must be refused cleanly.
///
/// Implementations are shared by all the threads of a kernel, hence `Send + Sync` and
/// `&self` receivers.
pub trait MemoryAccountant: Send + Sync {
    /// Reserves `bytes` of `kind` for `purpose`, a short name used in reports and error
    /// messages (for example `"pagerank ranks"`).
    ///
    /// # Errors
    ///
    /// [`InsufficientBudget`] if the reservation does not fit. Nothing is reserved in
    /// that case.
    fn try_reserve(
        &self,
        purpose: &'static str,
        kind: MemoryKind,
        bytes: u64,
    ) -> Result<(), InsufficientBudget>;

    /// Gives back `bytes` previously reserved with the same `purpose` and `kind`.
    fn release(&self, purpose: &'static str, kind: MemoryKind, bytes: u64);

    /// Bytes of [`MemoryKind::Anonymous`] memory that can still be reserved.
    fn available(&self) -> u64;
}

/// A reservation was refused because it does not fit in what is left of the budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InsufficientBudget {
    /// What the memory was for, as passed to [`MemoryAccountant::try_reserve`].
    pub purpose: &'static str,
    /// Bytes the caller asked for.
    pub needed: u64,
    /// Bytes that were available when it asked.
    pub available: u64,
}

impl InsufficientBudget {
    /// Describes a refused reservation. Accountants defined outside this crate build
    /// the error with this function, because the struct may gain fields.
    #[must_use]
    pub const fn new(purpose: &'static str, needed: u64, available: u64) -> Self {
        Self {
            purpose,
            needed,
            available,
        }
    }
}

impl fmt::Display for InsufficientBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} bytes needed for {} but only {} bytes of the memory budget are available",
            self.needed, self.purpose, self.available
        )
    }
}

impl Error for InsufficientBudget {}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// The smallest thing that satisfies the interface. It exists to show that the
    /// trait can be implemented, shared and used through `dyn`; it is not the
    /// accountant of issue #31. It limits anonymous memory and only adds up the rest.
    struct FixedBudget {
        available: AtomicU64,
        file_backed: AtomicU64,
    }

    impl FixedBudget {
        fn new(budget: u64) -> Self {
            Self {
                available: AtomicU64::new(budget),
                file_backed: AtomicU64::new(0),
            }
        }
    }

    impl MemoryAccountant for FixedBudget {
        fn try_reserve(
            &self,
            purpose: &'static str,
            kind: MemoryKind,
            bytes: u64,
        ) -> Result<(), InsufficientBudget> {
            if kind == MemoryKind::FileBacked {
                self.file_backed.fetch_add(bytes, Ordering::Relaxed);
                return Ok(());
            }
            let mut available = self.available.load(Ordering::Relaxed);
            loop {
                let Some(rest) = available.checked_sub(bytes) else {
                    return Err(InsufficientBudget::new(purpose, bytes, available));
                };
                match self.available.compare_exchange_weak(
                    available,
                    rest,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return Ok(()),
                    Err(current) => available = current,
                }
            }
        }

        fn release(&self, _purpose: &'static str, kind: MemoryKind, bytes: u64) {
            if kind == MemoryKind::FileBacked {
                self.file_backed.fetch_sub(bytes, Ordering::Relaxed);
            } else {
                self.available.fetch_add(bytes, Ordering::Relaxed);
            }
        }

        fn available(&self) -> u64 {
            self.available.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn a_refused_reservation_reports_the_shortfall_and_reserves_nothing() {
        let budget = FixedBudget::new(100);
        let accountant: &dyn MemoryAccountant = &budget;

        accountant
            .try_reserve("ranks", MemoryKind::Anonymous, 60)
            .unwrap();
        assert_eq!(accountant.available(), 40);

        let refused = accountant
            .try_reserve("frontier", MemoryKind::Anonymous, 41)
            .unwrap_err();
        assert_eq!(refused, InsufficientBudget::new("frontier", 41, 40));
        assert_eq!(accountant.available(), 40);

        accountant.release("ranks", MemoryKind::Anonymous, 60);
        accountant
            .try_reserve("frontier", MemoryKind::Anonymous, 41)
            .unwrap();
        assert_eq!(accountant.available(), 59);
    }

    /// The interface can tell a mapped file from a heap buffer: an accountant may let
    /// the first exceed a budget that the second must respect.
    #[test]
    fn the_kind_of_memory_reaches_the_accountant() {
        let budget = FixedBudget::new(100);

        budget
            .try_reserve("mapped adjacency", MemoryKind::FileBacked, 1 << 40)
            .unwrap();
        assert_eq!(budget.available(), 100);
        assert_eq!(budget.file_backed.load(Ordering::Relaxed), 1 << 40);

        let refused = budget
            .try_reserve("decoded adjacency", MemoryKind::Anonymous, 1 << 40)
            .unwrap_err();
        assert_eq!(refused.purpose, "decoded adjacency");

        budget.release("mapped adjacency", MemoryKind::FileBacked, 1 << 40);
        assert_eq!(budget.file_backed.load(Ordering::Relaxed), 0);
        assert_eq!(budget.available(), 100);
    }

    /// More than a 32-bit `usize` can count: the request is refused, not truncated.
    #[test]
    fn requests_beyond_the_address_space_are_expressible() {
        let budget = FixedBudget::new(1 << 30);
        let refused = budget
            .try_reserve("offsets", MemoryKind::Anonymous, 1 << 40)
            .unwrap_err();
        assert_eq!(refused.needed, 1 << 40);
        assert_eq!(refused.available, 1 << 30);
    }

    #[test]
    fn the_error_reads_as_a_sentence() {
        let refused = InsufficientBudget::new("pagerank ranks", 4096, 1024);
        assert_eq!(
            refused.to_string(),
            "4096 bytes needed for pagerank ranks but only 1024 bytes of the memory budget are available"
        );
    }
}
