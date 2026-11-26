// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Memory accounting: the budget, reservations, and the system probe interface.
//!
//! The promise "never run out of memory" (DESIGN § 2, D11) is kept by asking before
//! allocating: every large buffer is reserved against the user's budget first, and a
//! request that does not fit is refused with a typed error while nothing has been
//! allocated yet. The only out-of-memory kill in the lab came from O(n) state that
//! nobody had counted.
//!
//! # How memory is meant to be held
//!
//! Kernels do not call [`MemoryAccountant::try_reserve`] and
//! [`MemoryAccountant::release`] themselves. They hold memory through types that own
//! both the buffer and its reservation, so that the two cannot drift apart:
//!
//! - [`SegVec`](crate::SegVec) for arrays: every byte of capacity it allocates is
//!   reserved first, growth reserves before it reallocates (including the moment when
//!   the old and the new buffer both exist), a clone is
//!   [`try_clone`](crate::SegVec::try_clone), and dropping it releases the reservation.
//! - [`ScratchPool`](crate::ScratchPool) for per-thread scratch: one buffer per
//!   concurrent chunk, reserved for the whole scope of the pool.
//! - [`Reservation`] for what neither type can hold (a hash table, a buffer owned by
//!   another library): the caller sizes it, and keeps the guard next to the thing it
//!   pays for. A reservation with no buffer behind it is headroom, and headroom hides
//!   untracked memory; prefer the two types above.
//!
//! The accountant of the engine is [`Budget`]. It limits anonymous memory, records
//! file-backed memory without limiting it (a mapped file is reclaimable page cache,
//! FINDINGS § B1), and keeps the current and peak bytes per purpose for reports.
//!
//! What the machine has (physical memory, a container's limit) is read outside this
//! crate, through [`MemoryProbe`]: the core does not read `/proc` or cgroups.

use std::error::Error;
use std::fmt;
use std::io;
use std::sync::Arc;

mod budget;

pub use budget::{Budget, MemoryReport, PurposeUsage};

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
/// `&self` receivers. [`Budget`] is the implementation the engine uses; code that
/// allocates goes through [`Memory`] and the guards built on it, never through these
/// methods directly (module documentation).
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

/// Why memory could not be obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoryError {
    /// The budget does not have room for it. Nothing was allocated.
    Budget(InsufficientBudget),
    /// The budget had room but the allocator refused, or the size cannot be addressed
    /// on this target (more than a 32-bit `usize` can count, or a single allocation
    /// above [`MAX_ALLOCATION_BYTES`](crate::MAX_ALLOCATION_BYTES)). Nothing is held:
    /// the reservation made for it has been released.
    AllocationFailed {
        /// What the memory was for.
        purpose: &'static str,
        /// Bytes of the allocation that failed.
        bytes: u64,
    },
}

impl From<InsufficientBudget> for MemoryError {
    fn from(refused: InsufficientBudget) -> Self {
        Self::Budget(refused)
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget(refused) => refused.fmt(f),
            Self::AllocationFailed { purpose, bytes } => write!(
                f,
                "the system could not allocate {bytes} bytes for {purpose}, within the memory budget"
            ),
        }
    }
}

impl Error for MemoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Budget(refused) => Some(refused),
            Self::AllocationFailed { .. } => None,
        }
    }
}

/// A shared handle on an accountant: what kernels receive, and what every
/// [`Reservation`], [`SegVec`](crate::SegVec) and [`ScratchPool`](crate::ScratchPool)
/// keeps so that it can give its memory back when dropped. Cloning it is cheap.
///
/// ```
/// use std::sync::Arc;
/// use everygraph_core::{Budget, Memory, SegVec};
///
/// let budget = Arc::new(Budget::new(1 << 20));
/// let memory = Memory::from(Arc::clone(&budget));
/// let ranks = SegVec::try_with_len(&memory, "ranks", 1000, 0.0_f32).unwrap();
/// assert_eq!(budget.used(), 4000);
/// drop(ranks);
/// assert_eq!(budget.used(), 0);
/// assert_eq!(budget.peak(), 4000);
/// ```
#[derive(Clone)]
pub struct Memory(Arc<dyn MemoryAccountant>);

impl Memory {
    /// A handle on `accountant`.
    #[must_use]
    pub fn new(accountant: Arc<dyn MemoryAccountant>) -> Self {
        Self(accountant)
    }

    /// A handle on a fresh [`Budget::unlimited`]: for tests and for callers that have
    /// no budget yet. What is reserved is still counted.
    #[must_use]
    pub fn unlimited() -> Self {
        Self(Arc::new(Budget::unlimited()))
    }

    /// Reserves `bytes` of `kind` for `purpose`, as a guard that gives them back when
    /// dropped.
    ///
    /// # Errors
    ///
    /// [`InsufficientBudget`] if it does not fit; nothing is reserved then.
    pub fn reserve(
        &self,
        purpose: &'static str,
        kind: MemoryKind,
        bytes: u64,
    ) -> Result<Reservation, InsufficientBudget> {
        let mut reservation = Reservation::empty(self, purpose, kind);
        reservation.try_grow(bytes)?;
        Ok(reservation)
    }

    /// Bytes of anonymous memory that can still be reserved.
    #[must_use]
    pub fn available(&self) -> u64 {
        self.0.available()
    }

    /// The accountant itself, for code that takes one by reference.
    #[must_use]
    pub fn accountant(&self) -> &dyn MemoryAccountant {
        &*self.0
    }
}

impl<A: MemoryAccountant + 'static> From<Arc<A>> for Memory {
    fn from(accountant: Arc<A>) -> Self {
        Self(accountant)
    }
}

impl fmt::Debug for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memory")
            .field("available", &self.available())
            .finish_non_exhaustive()
    }
}

/// Bytes reserved for one purpose, given back when the guard is dropped.
///
/// The guard can grow and shrink, so that it always states what its owner holds. It
/// cannot be copied or cloned: a second claim on the same memory is a new reservation.
#[must_use = "a reservation is given back as soon as it is dropped"]
#[derive(Debug)]
pub struct Reservation {
    memory: Memory,
    purpose: &'static str,
    kind: MemoryKind,
    bytes: u64,
}

impl Reservation {
    /// A reservation of zero bytes, to be grown as its owner allocates.
    pub fn empty(memory: &Memory, purpose: &'static str, kind: MemoryKind) -> Self {
        Self {
            memory: memory.clone(),
            purpose,
            kind,
            bytes: 0,
        }
    }

    /// Bytes held.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// What the memory is for.
    #[must_use]
    pub const fn purpose(&self) -> &'static str {
        self.purpose
    }

    /// What the memory is backed by.
    #[must_use]
    pub const fn kind(&self) -> MemoryKind {
        self.kind
    }

    /// The handle the reservation was made through.
    #[must_use]
    pub const fn memory(&self) -> &Memory {
        &self.memory
    }

    /// Reserves `more` bytes in addition to what is held.
    ///
    /// # Errors
    ///
    /// [`InsufficientBudget`] if they do not fit (with `needed` = `more`), or if the
    /// total would not fit in a `u64`; what was held before is still held.
    pub fn try_grow(&mut self, more: u64) -> Result<(), InsufficientBudget> {
        if more == 0 {
            return Ok(());
        }
        let Some(total) = self.bytes.checked_add(more) else {
            return Err(InsufficientBudget::new(
                self.purpose,
                more,
                self.memory.available(),
            ));
        };
        self.memory.0.try_reserve(self.purpose, self.kind, more)?;
        self.bytes = total;
        Ok(())
    }

    /// Gives back `less` bytes, or everything held if that is less.
    pub fn shrink(&mut self, less: u64) {
        let less = less.min(self.bytes);
        if less > 0 {
            self.memory.0.release(self.purpose, self.kind, less);
            self.bytes -= less;
        }
    }

    /// Makes the reservation exactly `bytes`, growing or shrinking it.
    ///
    /// # Errors
    ///
    /// As [`try_grow`](Self::try_grow) when it has to grow.
    pub fn try_resize(&mut self, bytes: u64) -> Result<(), InsufficientBudget> {
        if bytes >= self.bytes {
            self.try_grow(bytes - self.bytes)
        } else {
            self.shrink(self.bytes - bytes);
            Ok(())
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.shrink(self.bytes);
    }
}

/// What the machine can give, as read by a [`MemoryProbe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SystemMemory {
    /// Physical memory of the machine, in bytes.
    pub machine_total: u64,
    /// Memory the operating system could give now without swapping (Linux's
    /// `MemAvailable`), in bytes.
    pub machine_available: u64,
    /// The tightest limit of the container or control group the process runs in, if
    /// any, in bytes.
    pub limit: Option<u64>,
    /// Bytes still free under that limit (the limit minus what the group already
    /// uses, at the level that leaves the least). Equal to `limit` when unknown.
    pub limit_available: Option<u64>,
}

impl SystemMemory {
    /// Describes what a probe found. Probes outside this crate build the value with
    /// this function, because the struct may gain fields.
    #[must_use]
    pub const fn new(
        machine_total: u64,
        machine_available: u64,
        limit: Option<u64>,
        limit_available: Option<u64>,
    ) -> Self {
        Self {
            machine_total,
            machine_available,
            limit,
            limit_available,
        }
    }

    /// The most memory the process could ever have: the machine's, or the limit when
    /// it is lower. A budget above it is a usage error (ux-contract P-16).
    #[must_use]
    pub fn total(&self) -> u64 {
        self.limit
            .map_or(self.machine_total, |limit| limit.min(self.machine_total))
    }

    /// The memory free now: the machine's, or what the limit leaves when that is
    /// lower. A budget above it is honoured with a warning (P-17).
    #[must_use]
    pub fn available(&self) -> u64 {
        self.limit_available
            .or(self.limit)
            .map_or(self.machine_available, |free| {
                free.min(self.machine_available)
            })
    }
}

/// Reads how much memory the machine has. The core defines the interface and never
/// implements it: probing reads operating-system files (`/proc`, cgroups) or browser
/// APIs. The Linux probe is `everygraph_io::ProcMemoryProbe`.
pub trait MemoryProbe {
    /// Reads the machine's memory now.
    ///
    /// # Errors
    ///
    /// The probe could not read or understand what the system reports. A probe never
    /// guesses a value it could not read.
    fn probe(&self) -> io::Result<SystemMemory>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(limit: u64) -> (Arc<Budget>, Memory) {
        let budget = Arc::new(Budget::new(limit));
        let memory = Memory::from(Arc::clone(&budget));
        (budget, memory)
    }

    #[test]
    fn a_refused_reservation_reports_the_shortfall_and_reserves_nothing() {
        let (budget, memory) = budget(100);
        let ranks = memory.reserve("ranks", MemoryKind::Anonymous, 60).unwrap();
        assert_eq!(memory.available(), 40);

        let refused = memory
            .reserve("frontier", MemoryKind::Anonymous, 41)
            .unwrap_err();
        assert_eq!(refused, InsufficientBudget::new("frontier", 41, 40));
        assert_eq!(budget.used(), 60);

        drop(ranks);
        let frontier = memory
            .reserve("frontier", MemoryKind::Anonymous, 41)
            .unwrap();
        assert_eq!(memory.available(), 59);
        assert_eq!(frontier.bytes(), 41);
    }

    #[test]
    fn a_reservation_grows_shrinks_and_is_given_back_on_drop() {
        let (budget, memory) = budget(100);
        let mut held = memory.reserve("state", MemoryKind::Anonymous, 10).unwrap();
        held.try_grow(50).unwrap();
        assert_eq!((held.bytes(), budget.used()), (60, 60));

        // A refused growth keeps what was held.
        assert_eq!(
            held.try_grow(41).unwrap_err(),
            InsufficientBudget::new("state", 41, 40)
        );
        assert_eq!((held.bytes(), budget.used()), (60, 60));

        held.shrink(15);
        assert_eq!((held.bytes(), budget.used()), (45, 45));
        held.shrink(1000);
        assert_eq!((held.bytes(), budget.used()), (0, 0));

        held.try_resize(70).unwrap();
        held.try_resize(30).unwrap();
        assert_eq!((held.bytes(), budget.used()), (30, 30));
        drop(held);
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.peak(), 70);
    }

    /// File-backed memory is recorded, not limited: a mapping larger than the budget
    /// is accepted, and anonymous memory is still refused.
    #[test]
    fn file_backed_memory_is_recorded_but_not_limited() {
        let (budget, memory) = budget(100);
        let mapped = memory
            .reserve("mapped adjacency", MemoryKind::FileBacked, 1 << 40)
            .unwrap();
        assert_eq!(memory.available(), 100);
        assert_eq!(budget.report().file_backed, 1 << 40);

        let refused = memory
            .reserve("decoded adjacency", MemoryKind::Anonymous, 1 << 40)
            .unwrap_err();
        assert_eq!(refused.purpose, "decoded adjacency");

        drop(mapped);
        let report = budget.report();
        assert_eq!((report.file_backed, report.file_backed_peak), (0, 1 << 40));
        assert_eq!(budget.peak(), 0);
    }

    /// More than a 32-bit `usize` can count: the request is refused, not truncated.
    #[test]
    fn requests_beyond_the_address_space_are_expressible() {
        let (_, memory) = budget(1 << 30);
        let refused = memory
            .reserve("offsets", MemoryKind::Anonymous, 1 << 40)
            .unwrap_err();
        assert_eq!(
            refused,
            InsufficientBudget::new("offsets", 1 << 40, 1 << 30)
        );
    }

    /// A total that overflows `u64` is a refusal, not a wrap-around, even on an
    /// unlimited budget.
    #[test]
    fn a_reservation_that_would_overflow_is_refused() {
        let memory = Memory::unlimited();
        let mut held = memory
            .reserve("state", MemoryKind::Anonymous, u64::MAX - 1)
            .unwrap();
        assert!(held.try_grow(2).is_err());
        assert_eq!(held.bytes(), u64::MAX - 1);
    }

    #[test]
    fn the_errors_read_as_sentences() {
        let refused = InsufficientBudget::new("pagerank ranks", 4096, 1024);
        assert_eq!(
            refused.to_string(),
            "4096 bytes needed for pagerank ranks but only 1024 bytes of the memory budget are available"
        );
        assert_eq!(MemoryError::from(refused).to_string(), refused.to_string());
        assert_eq!(
            MemoryError::AllocationFailed {
                purpose: "ranks",
                bytes: 8
            }
            .to_string(),
            "the system could not allocate 8 bytes for ranks, within the memory budget"
        );
    }

    #[test]
    fn system_memory_takes_the_tighter_of_machine_and_limit() {
        let gib = 1 << 30;
        let free = SystemMemory::new(64 * gib, 40 * gib, None, None);
        assert_eq!((free.total(), free.available()), (64 * gib, 40 * gib));

        let scoped = SystemMemory::new(64 * gib, 40 * gib, Some(8 * gib), Some(6 * gib));
        assert_eq!((scoped.total(), scoped.available()), (8 * gib, 6 * gib));

        let loose = SystemMemory::new(64 * gib, 40 * gib, Some(128 * gib), None);
        assert_eq!((loose.total(), loose.available()), (64 * gib, 40 * gib));
    }
}
