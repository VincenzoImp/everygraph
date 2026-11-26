// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`Budget`], the accountant of the engine.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{InsufficientBudget, MemoryAccountant, MemoryKind};

/// Distinct (purpose, kind) pairs tracked one by one; any further pair is added to the
/// shared "other purposes" entry of its kind. A kernel names a handful.
const PURPOSE_SLOTS: usize = 64;

/// The name the report gives to the pairs beyond [`PURPOSE_SLOTS`].
const OTHER_PURPOSES: &str = "(other purposes)";

/// A fixed memory budget: limits anonymous memory, records file-backed memory, and
/// keeps current and peak bytes overall and per purpose.
///
/// Everything is lock-free: a reservation is a few atomic operations, and nothing here
/// can block, so the accountant can be used from a browser's main thread.
///
/// # Panics
///
/// [`release`](MemoryAccountant::release) panics when it is asked to give back more
/// anonymous memory than is reserved: that is a bug in the caller, and the count would
/// be wrong from then on. The guards of this crate cannot cause it.
#[derive(Debug)]
pub struct Budget {
    limit: u64,
    used: AtomicU64,
    peak: AtomicU64,
    file_backed: AtomicU64,
    file_backed_peak: AtomicU64,
    purposes: Box<[Slot]>,
    /// The pairs beyond the table: anonymous first, file-backed second. Labelled when
    /// the budget is made, so that no reservation ever initializes a shared label.
    other: [Slot; 2],
}

/// Counters of one (purpose, kind) pair. A slot is claimed by writing its key with a
/// compare-and-swap; only the winner sets the label, so `OnceLock::set` never waits.
#[derive(Debug)]
struct Slot {
    key: AtomicU64,
    label: OnceLock<(&'static str, MemoryKind)>,
    current: AtomicU64,
    peak: AtomicU64,
}

impl Slot {
    fn new() -> Self {
        Self {
            key: AtomicU64::new(0),
            label: OnceLock::new(),
            current: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        }
    }

    fn labelled(purpose: &'static str, kind: MemoryKind) -> Self {
        Self {
            label: OnceLock::from((purpose, kind)),
            ..Self::new()
        }
    }

    fn add(&self, bytes: u64) {
        let now = saturating_add(&self.current, bytes);
        self.peak.fetch_max(now, Ordering::Relaxed);
    }

    fn sub(&self, bytes: u64) {
        let _ = update(&self.current, |current| Some(current.saturating_sub(bytes)));
    }
}

/// Replaces the value of `counter` by `f(value)` atomically, unless `f` returns `None`.
/// Returns the previous value, or `Err` with the current one. (`AtomicU64::fetch_update`
/// does this, but is deprecated on current Rust under a name the minimum supported
/// version does not have.)
fn update(counter: &AtomicU64, f: impl Fn(u64) -> Option<u64>) -> Result<u64, u64> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let new = f(current).ok_or(current)?;
        match counter.compare_exchange_weak(current, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(previous) => return Ok(previous),
            Err(actual) => current = actual,
        }
    }
}

/// Adds `bytes` to `counter` without wrapping; returns the new value.
fn saturating_add(counter: &AtomicU64, bytes: u64) -> u64 {
    let previous = update(counter, |current| Some(current.saturating_add(bytes)))
        .unwrap_or_else(|current| current);
    previous.saturating_add(bytes)
}

/// FNV-1a of the purpose, with the kind folded in; never 0, which marks a free slot.
/// Two pairs with the same key would share counters; with 64-bit keys and a few dozen
/// purposes that does not happen in practice, and it would only merge two lines of a
/// report, never change what is limited.
fn key(purpose: &str, kind: MemoryKind) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in purpose.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if kind == MemoryKind::FileBacked {
        hash = !hash;
    }
    hash.max(1)
}

/// What a [`Budget`] has seen.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MemoryReport {
    /// The budget for anonymous memory, in bytes (`u64::MAX` when unlimited).
    pub limit: u64,
    /// Anonymous bytes reserved now.
    pub used: u64,
    /// Most anonymous bytes reserved at any one time.
    pub peak: u64,
    /// File-backed bytes recorded now.
    pub file_backed: u64,
    /// Most file-backed bytes recorded at any one time.
    pub file_backed_peak: u64,
    /// Per purpose, sorted by purpose and kind.
    pub purposes: Vec<PurposeUsage>,
}

/// Bytes held for one purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PurposeUsage {
    /// The purpose, as given to the reservations.
    pub purpose: &'static str,
    /// What the memory is backed by.
    pub kind: MemoryKind,
    /// Bytes held now.
    pub current: u64,
    /// Most bytes held at any one time for this purpose alone. The peaks of different
    /// purposes need not have happened at the same moment, so they do not add up to
    /// [`MemoryReport::peak`].
    pub peak: u64,
}

impl Budget {
    /// A budget of `limit` bytes of anonymous memory.
    #[must_use]
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            used: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            file_backed: AtomicU64::new(0),
            file_backed_peak: AtomicU64::new(0),
            purposes: (0..PURPOSE_SLOTS).map(|_| Slot::new()).collect(),
            other: [
                Slot::labelled(OTHER_PURPOSES, MemoryKind::Anonymous),
                Slot::labelled(OTHER_PURPOSES, MemoryKind::FileBacked),
            ],
        }
    }

    /// A budget that refuses nothing (`u64::MAX` bytes) and still counts.
    #[must_use]
    pub fn unlimited() -> Self {
        Self::new(u64::MAX)
    }

    /// The limit on anonymous memory, in bytes.
    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    /// Anonymous bytes reserved now.
    #[must_use]
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    /// Most anonymous bytes reserved at any one time.
    #[must_use]
    pub fn peak(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }

    /// Everything the budget has seen, for reports and for comparing the plan's
    /// estimate with what was reserved (ux-contract P-9).
    #[must_use]
    pub fn report(&self) -> MemoryReport {
        let mut purposes: Vec<PurposeUsage> = self
            .purposes
            .iter()
            .chain(
                self.other
                    .iter()
                    .filter(|slot| slot.peak.load(Ordering::Relaxed) > 0),
            )
            .filter_map(|slot| {
                let &(purpose, kind) = slot.label.get()?;
                Some(PurposeUsage {
                    purpose,
                    kind,
                    current: slot.current.load(Ordering::Relaxed),
                    peak: slot.peak.load(Ordering::Relaxed),
                })
            })
            .collect();
        purposes.sort_by(|a, b| {
            (a.purpose, a.kind == MemoryKind::FileBacked)
                .cmp(&(b.purpose, b.kind == MemoryKind::FileBacked))
        });
        MemoryReport {
            limit: self.limit,
            used: self.used(),
            peak: self.peak(),
            file_backed: self.file_backed.load(Ordering::Relaxed),
            file_backed_peak: self.file_backed_peak.load(Ordering::Relaxed),
            purposes,
        }
    }

    /// The counters of (`purpose`, `kind`), claiming a slot the first time.
    fn slot(&self, purpose: &'static str, kind: MemoryKind) -> &Slot {
        let key = key(purpose, kind);
        for slot in &*self.purposes {
            let found = match slot.key.load(Ordering::Acquire) {
                0 => match slot
                    .key
                    .compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire)
                {
                    Ok(_) => {
                        // Only the winner of the swap gets here: `set` cannot wait.
                        let _ = slot.label.set((purpose, kind));
                        true
                    }
                    Err(other) => other == key,
                },
                other => other == key,
            };
            if found {
                return slot;
            }
        }
        &self.other[usize::from(kind == MemoryKind::FileBacked)]
    }
}

impl MemoryAccountant for Budget {
    fn try_reserve(
        &self,
        purpose: &'static str,
        kind: MemoryKind,
        bytes: u64,
    ) -> Result<(), InsufficientBudget> {
        if kind == MemoryKind::FileBacked {
            let now = saturating_add(&self.file_backed, bytes);
            self.file_backed_peak.fetch_max(now, Ordering::Relaxed);
        } else {
            let previous = update(&self.used, |used| {
                used.checked_add(bytes).filter(|&total| total <= self.limit)
            })
            .map_err(|used| {
                InsufficientBudget::new(purpose, bytes, self.limit.saturating_sub(used))
            })?;
            self.peak.fetch_max(previous + bytes, Ordering::Relaxed);
        }
        self.slot(purpose, kind).add(bytes);
        Ok(())
    }

    fn release(&self, purpose: &'static str, kind: MemoryKind, bytes: u64) {
        if kind == MemoryKind::FileBacked {
            let _ = update(&self.file_backed, |current| {
                Some(current.saturating_sub(bytes))
            });
        } else {
            let released = update(&self.used, |used| used.checked_sub(bytes));
            assert!(
                released.is_ok(),
                "{bytes} bytes of {purpose} released, more than the budget has reserved"
            );
        }
        self.slot(purpose, kind).sub(bytes);
    }

    fn available(&self) -> u64 {
        self.limit.saturating_sub(self.used())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_keeps_current_and_peak_per_purpose() {
        let budget = Budget::new(1000);
        budget
            .try_reserve("ranks", MemoryKind::Anonymous, 400)
            .unwrap();
        budget
            .try_reserve("frontier", MemoryKind::Anonymous, 300)
            .unwrap();
        budget.release("frontier", MemoryKind::Anonymous, 300);
        budget
            .try_reserve("frontier", MemoryKind::Anonymous, 100)
            .unwrap();
        budget
            .try_reserve("ranks", MemoryKind::FileBacked, 5000)
            .unwrap();
        assert!(
            budget
                .try_reserve("scratch", MemoryKind::Anonymous, 501)
                .is_err()
        );

        let report = budget.report();
        assert_eq!((report.limit, report.used, report.peak), (1000, 500, 700));
        assert_eq!((report.file_backed, report.file_backed_peak), (5000, 5000));
        let lines: Vec<_> = report
            .purposes
            .iter()
            .map(|p| (p.purpose, p.kind, p.current, p.peak))
            .collect();
        assert_eq!(
            lines,
            [
                ("frontier", MemoryKind::Anonymous, 100, 300),
                ("ranks", MemoryKind::Anonymous, 400, 400),
                ("ranks", MemoryKind::FileBacked, 5000, 5000),
            ],
            "a refused reservation leaves no line"
        );
    }

    #[test]
    fn purposes_beyond_the_table_are_counted_together() {
        #[rustfmt::skip]
        let names: [&str; PURPOSE_SLOTS + 2] = [
            "p00", "p01", "p02", "p03", "p04", "p05", "p06", "p07", "p08", "p09",
            "p10", "p11", "p12", "p13", "p14", "p15", "p16", "p17", "p18", "p19",
            "p20", "p21", "p22", "p23", "p24", "p25", "p26", "p27", "p28", "p29",
            "p30", "p31", "p32", "p33", "p34", "p35", "p36", "p37", "p38", "p39",
            "p40", "p41", "p42", "p43", "p44", "p45", "p46", "p47", "p48", "p49",
            "p50", "p51", "p52", "p53", "p54", "p55", "p56", "p57", "p58", "p59",
            "p60", "p61", "p62", "p63", "p64", "p65",
        ];
        let budget = Budget::unlimited();
        for &name in &names {
            budget.try_reserve(name, MemoryKind::Anonymous, 1).unwrap();
        }
        let report = budget.report();
        assert_eq!(report.purposes.len(), PURPOSE_SLOTS + 1);
        let other = report
            .purposes
            .iter()
            .find(|p| p.purpose == OTHER_PURPOSES)
            .unwrap();
        assert_eq!((other.current, other.peak), (2, 2));
        assert_eq!(other.kind, MemoryKind::Anonymous);

        // A file-backed pair beyond the table has its own line, not the anonymous one.
        budget
            .try_reserve("mapped", MemoryKind::FileBacked, 1 << 40)
            .unwrap();
        let report = budget.report();
        let others: Vec<_> = report
            .purposes
            .iter()
            .filter(|p| p.purpose == OTHER_PURPOSES)
            .map(|p| (p.kind, p.current))
            .collect();
        assert_eq!(
            others,
            [
                (MemoryKind::Anonymous, 2),
                (MemoryKind::FileBacked, 1 << 40)
            ]
        );
        budget.release("mapped", MemoryKind::FileBacked, 1 << 40);
        assert_eq!(report.used, crate::widen(names.len()));
        for &name in &names {
            budget.release(name, MemoryKind::Anonymous, 1);
        }
        assert_eq!(budget.report().used, 0);
    }

    #[test]
    #[should_panic(expected = "more than the budget has reserved")]
    fn releasing_more_than_reserved_is_a_bug() {
        let budget = Budget::new(100);
        budget.try_reserve("a", MemoryKind::Anonymous, 10).unwrap();
        budget.release("a", MemoryKind::Anonymous, 11);
    }

    /// Reservations from many chunks at once never exceed the limit, and the books
    /// balance at the end.
    #[test]
    fn concurrent_reservations_respect_the_limit() {
        use std::num::NonZeroUsize;
        use std::sync::atomic::AtomicU64;

        let budget = Budget::new(1000);
        let granted = AtomicU64::new(0);
        crate::par::for_each_range(64, NonZeroUsize::new(1).unwrap(), |range| {
            for _ in range {
                if budget
                    .try_reserve("chunk", MemoryKind::Anonymous, 100)
                    .is_ok()
                {
                    granted.fetch_add(1, Ordering::Relaxed);
                    assert!(budget.used() <= 1000);
                    budget.release("chunk", MemoryKind::Anonymous, 100);
                }
            }
        });
        assert!(granted.load(Ordering::Relaxed) >= 1);
        assert_eq!(budget.used(), 0);
        assert!(budget.peak() <= 1000);
    }
}
