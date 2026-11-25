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
//! accountant as a parameter. The accountant itself (budget, peak tracking, anonymous
//! versus file-backed memory, reservation guards) is issue #31.

use std::error::Error;
use std::fmt;

/// Keeps count of the memory a computation has claimed out of a fixed budget.
///
/// Sizes are `u64` on every target: a plan is checked before anything is allocated, and
/// the amount a graph would need can exceed what a 32-bit `usize` can express — which
/// is exactly the request that must be refused cleanly.
///
/// Implementations are shared by all the threads of a kernel, hence `Send + Sync` and
/// `&self` receivers.
pub trait MemoryAccountant: Send + Sync {
    /// Reserves `bytes` for `purpose`, a short name used in reports and error messages
    /// (for example `"pagerank ranks"`).
    ///
    /// # Errors
    ///
    /// [`InsufficientBudget`] if fewer than `bytes` are available. Nothing is reserved
    /// in that case.
    fn try_reserve(&self, purpose: &'static str, bytes: u64) -> Result<(), InsufficientBudget>;

    /// Gives back `bytes` previously reserved for `purpose`.
    fn release(&self, purpose: &'static str, bytes: u64);

    /// Bytes that can still be reserved.
    fn available(&self) -> u64;
}

/// A reservation was refused because it does not fit in what is left of the budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsufficientBudget {
    /// Bytes the caller asked for.
    pub needed: u64,
    /// Bytes that were available when it asked.
    pub available: u64,
}

impl fmt::Display for InsufficientBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} bytes needed but only {} bytes of the memory budget are available",
            self.needed, self.available
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
    /// accountant of issue #31.
    struct FixedBudget {
        available: AtomicU64,
    }

    impl MemoryAccountant for FixedBudget {
        fn try_reserve(
            &self,
            _purpose: &'static str,
            bytes: u64,
        ) -> Result<(), InsufficientBudget> {
            let mut available = self.available.load(Ordering::Relaxed);
            loop {
                let Some(rest) = available.checked_sub(bytes) else {
                    return Err(InsufficientBudget {
                        needed: bytes,
                        available,
                    });
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

        fn release(&self, _purpose: &'static str, bytes: u64) {
            self.available.fetch_add(bytes, Ordering::Relaxed);
        }

        fn available(&self) -> u64 {
            self.available.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn a_refused_reservation_reports_the_shortfall_and_reserves_nothing() {
        let budget = FixedBudget {
            available: AtomicU64::new(100),
        };
        let accountant: &dyn MemoryAccountant = &budget;

        accountant.try_reserve("ranks", 60).unwrap();
        assert_eq!(accountant.available(), 40);

        let refused = accountant.try_reserve("frontier", 41).unwrap_err();
        assert_eq!(
            refused,
            InsufficientBudget {
                needed: 41,
                available: 40
            }
        );
        assert_eq!(accountant.available(), 40);

        accountant.release("ranks", 60);
        accountant.try_reserve("frontier", 41).unwrap();
        assert_eq!(accountant.available(), 59);
    }

    /// More than a 32-bit `usize` can count: the request is refused, not truncated.
    #[test]
    fn requests_beyond_the_address_space_are_expressible() {
        let budget = FixedBudget {
            available: AtomicU64::new(1 << 30),
        };
        let refused = budget.try_reserve("offsets", 1 << 40).unwrap_err();
        assert_eq!(refused.needed, 1 << 40);
        assert_eq!(refused.available, 1 << 30);
    }

    #[test]
    fn the_error_reads_as_a_sentence() {
        let refused = InsufficientBudget {
            needed: 4096,
            available: 1024,
        };
        assert_eq!(
            refused.to_string(),
            "4096 bytes needed but only 1024 bytes of the memory budget are available"
        );
    }
}
