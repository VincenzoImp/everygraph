// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A test accountant: a fixed budget of anonymous memory, with the peak reserved.
//! (The production accountant is issue #31; this one only has to obey the trait.)

use std::sync::atomic::{AtomicU64, Ordering};

use everygraph_core::{InsufficientBudget, MemoryAccountant, MemoryKind};

pub(crate) struct Budget {
    total: u64,
    used: AtomicU64,
    peak: AtomicU64,
}

impl Budget {
    pub(crate) fn new(total: u64) -> Self {
        Self {
            total,
            used: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        }
    }

    pub(crate) fn total(&self) -> u64 {
        self.total
    }

    pub(crate) fn peak(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }
}

impl MemoryAccountant for Budget {
    fn try_reserve(
        &self,
        purpose: &'static str,
        kind: MemoryKind,
        bytes: u64,
    ) -> Result<(), InsufficientBudget> {
        if kind != MemoryKind::Anonymous {
            return Ok(());
        }
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            let after = used.checked_add(bytes).filter(|a| *a <= self.total);
            let Some(after) = after else {
                return Err(InsufficientBudget::new(purpose, bytes, self.total - used));
            };
            match self
                .used
                .compare_exchange_weak(used, after, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.peak.fetch_max(after, Ordering::Relaxed);
                    return Ok(());
                }
                Err(now) => used = now,
            }
        }
    }

    fn release(&self, _purpose: &'static str, kind: MemoryKind, bytes: u64) {
        if kind == MemoryKind::Anonymous {
            let before = self.used.fetch_sub(bytes, Ordering::Relaxed);
            assert!(before >= bytes, "released more than reserved");
        }
    }

    fn available(&self) -> u64 {
        self.total - self.used.load(Ordering::Relaxed)
    }
}
