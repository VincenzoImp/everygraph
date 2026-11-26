// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`Ledger`], the accountant of a measured run: a [`Budget`] that tells the counting
//! allocator what it holds and which phase reserves, counts its reservations, and can
//! refuse the k-th one (DESIGN D2: every allocation is fallible, so every refusal must
//! end in a typed error).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use everygraph_core::{Budget, InsufficientBudget, MemoryAccountant, MemoryKind, Pages};

use crate::counting;

/// A budget, observed (module documentation).
#[derive(Debug)]
pub struct Ledger {
    budget: Budget,
    fail_at: Option<u64>,
    calls: AtomicU64,
    refused: OnceLock<&'static str>,
}

impl Ledger {
    /// A ledger of `limit` bytes (`None`: unlimited) whose buffers come from `pages`;
    /// with `fail_at = Some(k)`, the k-th reservation (from 1) is refused whatever the
    /// budget says.
    #[must_use]
    pub fn new(limit: Option<u64>, pages: Pages, fail_at: Option<u64>) -> Self {
        let budget = limit.map_or_else(Budget::unlimited, Budget::new);
        Self {
            budget: budget.with_pages(pages),
            fail_at,
            calls: AtomicU64::new(0),
            refused: OnceLock::new(),
        }
    }

    /// The budget underneath, for its peak and its report.
    #[must_use]
    pub const fn budget(&self) -> &Budget {
        &self.budget
    }

    /// Reservations asked for so far, refused ones included.
    #[must_use]
    pub fn reservations(&self) -> u64 {
        self.calls.load(Relaxed)
    }

    /// The purpose of the first refused reservation, if any was refused.
    #[must_use]
    pub fn refused(&self) -> Option<&'static str> {
        self.refused.get().copied()
    }
}

impl MemoryAccountant for Ledger {
    fn try_reserve(
        &self,
        purpose: &'static str,
        kind: MemoryKind,
        bytes: u64,
    ) -> Result<(), InsufficientBudget> {
        let k = self.calls.fetch_add(1, Relaxed) + 1;
        let result = if self.fail_at == Some(k) {
            Err(InsufficientBudget::new(purpose, bytes, 0))
        } else {
            self.budget.try_reserve(purpose, kind, bytes)
        };
        match result {
            Ok(()) if kind == MemoryKind::Anonymous => {
                counting::ledger(self.budget.used());
                counting::enter(purpose);
            }
            Ok(()) => {}
            Err(_) => {
                let _ = self.refused.set(purpose);
            }
        }
        result
    }

    fn release(&self, purpose: &'static str, kind: MemoryKind, bytes: u64) {
        self.budget.release(purpose, kind, bytes);
        counting::ledger(self.budget.used());
    }

    fn available(&self) -> u64 {
        self.budget.available()
    }

    fn pages(&self) -> Pages {
        self.budget.pages()
    }
}
