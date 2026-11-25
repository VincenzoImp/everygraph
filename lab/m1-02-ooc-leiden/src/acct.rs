//! A stub of the memory accountant (DESIGN D11; the real one is issue #31), implementing the
//! product's interface `everygraph_core::MemoryAccountant`.
//!
//! Every large buffer of the Leiden run is reserved here before it is allocated: the O(n)
//! state, the aggregation arrays, the bucket buffers, the per-bucket sort buffers, the
//! offsets index of each spilled quotient graph, the evaluation arrays. A mandatory
//! reservation that does not fit ends the run with exit status 3 and a JSON line naming the
//! purpose and the shortfall, before anything is allocated (the planner's "refuse with the
//! minimum budget"). An optional one (bucket chunks kept in RAM) falls back to disk.
//!
//! Only anonymous memory is limited; file-backed memory (the mmapped level-0 store and the
//! spilled quotient graphs) is recorded but not counted, as the interface documents. Also
//! reserved: the per-thread hash tables and decode buffers of local moving and refinement
//! (`scratch_bytes`, worst case of the largest degrees), the per-thread list buffers and the
//! split-community assembly of the aggregation, the next level's initial labels. Not reserved:
//! thread stacks, small vectors (O(threads) or O(buckets)), allocator slack, and the whole
//! member-order aggregation that `--check-agg` runs for comparison (a test mode). The run
//! reports the accountant's peak next to the measured anonymous peak so the gap can be read.
use everygraph_core::{InsufficientBudget, MemoryAccountant, MemoryKind};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

pub struct Stub {
    budget: AtomicU64,
    used: AtomicU64,
    peak: AtomicU64,
    file_backed: AtomicU64,
    file_backed_peak: AtomicU64,
    /// purpose -> (current, peak)
    by_purpose: Mutex<BTreeMap<&'static str, (u64, u64)>>,
    refused_optional: AtomicU64,
}

impl Stub {
    fn new(budget: u64) -> Stub {
        Stub {
            budget: AtomicU64::new(budget),
            used: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            file_backed: AtomicU64::new(0),
            file_backed_peak: AtomicU64::new(0),
            by_purpose: Mutex::new(BTreeMap::new()),
            refused_optional: AtomicU64::new(0),
        }
    }
    pub fn budget(&self) -> u64 {
        self.budget.load(Relaxed)
    }
    pub fn used(&self) -> u64 {
        self.used.load(Relaxed)
    }
    pub fn report(&self) -> serde_json::Value {
        let bp = self.by_purpose.lock().unwrap();
        let mut peaks = serde_json::Map::new();
        for (k, &(_, p)) in bp.iter() {
            peaks.insert(k.to_string(), serde_json::json!(p));
        }
        serde_json::json!({
            "budget_bytes": self.budget(),
            "peak_bytes": self.peak.load(Relaxed),
            "used_at_end_bytes": self.used(),
            "file_backed_peak_bytes": self.file_backed_peak.load(Relaxed),
            "optional_reservations_refused": self.refused_optional.load(Relaxed),
            "peak_by_purpose_bytes": peaks,
        })
    }
    fn note(&self, purpose: &'static str, delta: i64) {
        let mut bp = self.by_purpose.lock().unwrap();
        let e = bp.entry(purpose).or_insert((0, 0));
        e.0 = (e.0 as i64 + delta).max(0) as u64;
        e.1 = e.1.max(e.0);
    }
}

impl MemoryAccountant for Stub {
    fn try_reserve(&self, purpose: &'static str, kind: MemoryKind, bytes: u64) -> Result<(), InsufficientBudget> {
        if kind == MemoryKind::FileBacked {
            let now = self.file_backed.fetch_add(bytes, Relaxed) + bytes;
            self.file_backed_peak.fetch_max(now, Relaxed);
            return Ok(());
        }
        let budget = self.budget();
        let mut cur = self.used.load(Relaxed);
        loop {
            if cur + bytes > budget {
                return Err(InsufficientBudget::new(purpose, bytes, budget.saturating_sub(cur)));
            }
            match self.used.compare_exchange_weak(cur, cur + bytes, Relaxed, Relaxed) {
                Ok(_) => break,
                Err(c) => cur = c,
            }
        }
        self.peak.fetch_max(cur + bytes, Relaxed);
        self.note(purpose, bytes as i64);
        Ok(())
    }
    fn release(&self, purpose: &'static str, kind: MemoryKind, bytes: u64) {
        if kind == MemoryKind::FileBacked {
            self.file_backed.fetch_sub(bytes, Relaxed);
            return;
        }
        self.used.fetch_sub(bytes, Relaxed);
        self.note(purpose, -(bytes as i64));
    }
    fn available(&self) -> u64 {
        self.budget().saturating_sub(self.used())
    }
}

static ACCT: OnceLock<Stub> = OnceLock::new();

/// Sets the budget once, at start-up (u64::MAX = unlimited: the in-RAM reference still records
/// what it would have reserved).
pub fn init(budget: u64) {
    let _ = ACCT.set(Stub::new(budget));
}
pub fn acct() -> &'static Stub {
    ACCT.get_or_init(|| Stub::new(u64::MAX))
}
/// Tests that use the global accountant hold this lock.
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());
/// Tests: change the budget of the global accountant (hold TEST_LOCK).
#[cfg(test)]
pub fn set_budget_for_tests(b: u64) {
    acct().budget.store(b, Relaxed);
}

/// A reservation, given back when dropped.
#[must_use]
pub struct Res {
    purpose: &'static str,
    kind: MemoryKind,
    bytes: u64,
}
impl Res {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    /// Gives back part of the reservation (a buffer that shrank).
    pub fn shrink_to(&mut self, bytes: u64) {
        if bytes < self.bytes {
            acct().release(self.purpose, self.kind, self.bytes - bytes);
            self.bytes = bytes;
        }
    }
    /// Makes the reservation exactly `bytes`: gives back the excess, or reserves the shortfall
    /// (mandatory: refuses the run if it does not fit).
    pub fn fit(&mut self, bytes: u64) {
        if bytes < self.bytes {
            self.shrink_to(bytes);
        } else if bytes > self.bytes {
            let more = res(self.purpose, bytes - self.bytes);
            self.bytes = bytes;
            std::mem::forget(more); // now part of this reservation
        }
    }
    pub fn empty(purpose: &'static str) -> Res {
        Res { purpose, kind: MemoryKind::Anonymous, bytes: 0 }
    }
}
impl Drop for Res {
    fn drop(&mut self) {
        if self.bytes > 0 {
            acct().release(self.purpose, self.kind, self.bytes);
        }
    }
}

/// Mandatory reservation: refuses the run (exit 3) if it does not fit.
pub fn res(purpose: &'static str, bytes: u64) -> Res {
    match acct().try_reserve(purpose, MemoryKind::Anonymous, bytes) {
        Ok(()) => Res { purpose, kind: MemoryKind::Anonymous, bytes },
        Err(e) => {
            let a = acct();
            println!(
                "{}",
                serde_json::json!({"status": "refused", "error": e.to_string(), "purpose": purpose,
                    "needed_bytes": bytes, "available_bytes": e.available, "budget_bytes": a.budget(), "used_bytes": a.used()})
            );
            eprintln!("refused: {e}");
            std::process::exit(3);
        }
    }
}
/// Optional reservation (the caller has a fallback, e.g. spilling to disk).
pub fn try_res(purpose: &'static str, bytes: u64) -> Option<Res> {
    match acct().try_reserve(purpose, MemoryKind::Anonymous, bytes) {
        Ok(()) => Some(Res { purpose, kind: MemoryKind::Anonymous, bytes }),
        Err(_) => {
            acct().refused_optional.fetch_add(1, Relaxed);
            None
        }
    }
}
/// File-backed memory (a mapping): recorded, never refused.
pub fn file_backed(purpose: &'static str, bytes: u64) -> Res {
    let _ = acct().try_reserve(purpose, MemoryKind::FileBacked, bytes);
    Res { purpose, kind: MemoryKind::FileBacked, bytes }
}
/// Bytes of a Vec<T> of `len` elements.
pub fn sz<T>(len: usize) -> u64 {
    (len * std::mem::size_of::<T>()) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stub_limits_anonymous_and_records_file_backed() {
        let s = Stub::new(100);
        assert!(s.try_reserve("a", MemoryKind::Anonymous, 60).is_ok());
        assert!(s.try_reserve("b", MemoryKind::Anonymous, 41).is_err());
        assert!(s.try_reserve("f", MemoryKind::FileBacked, 1 << 40).is_ok());
        assert_eq!(s.available(), 40);
        s.release("a", MemoryKind::Anonymous, 60);
        assert!(s.try_reserve("b", MemoryKind::Anonymous, 41).is_ok());
        assert_eq!(s.peak.load(Relaxed), 60);
    }
}
