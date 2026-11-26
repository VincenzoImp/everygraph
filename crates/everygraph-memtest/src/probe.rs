// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One measured run ([`measure`]) and what is built on it: the growth of untracked heap
//! with the graph's size ([`slope`]), the smallest budget that succeeds
//! ([`Boundary`]), and the sweep that refuses each reservation in turn
//! ([`refuse_each`]). Challenge 3 of `docs/research/sota-testing.md`, items 1, 4 and 5.

use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use everygraph_core::{Memory, Pages, par};

use crate::counting;
use crate::ledger::Ledger;
use crate::ops::{Failure, THREADS};

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It completed.
    Done,
    /// It returned the error that says the budget is too small: the typed refusal.
    Refused(String),
    /// It returned another error.
    Failed(String),
    /// It panicked.
    Panicked(String),
}

/// What one run used.
#[derive(Clone, Debug)]
pub struct Run {
    /// How it ended.
    pub outcome: Outcome,
    /// The heap's peak above what was in use at the start, in requested bytes.
    pub heap_peak: u64,
    /// The ledger's anonymous peak.
    pub ledger_peak: u64,
    /// Heap still in use at the end above the start (negative: freed more).
    pub heap_left: i64,
    /// Anonymous bytes the ledger still held at the end.
    pub ledger_left: u64,
    /// Reservations asked for.
    pub reservations: u64,
    /// The most heap beyond the ledger, by phase.
    pub phases: Vec<(&'static str, u64)>,
    /// Files left in the scratch directory.
    pub scratch_left: usize,
}

impl Run {
    /// The most, over the run, of the heap in use above the start minus the ledger's
    /// anonymous bytes at the same moment, taken at each allocation. So `heap_peak ≤
    /// ledger_peak + untracked()`. A reservation held without its buffer (headroom)
    /// counts as covering heap, so heap that nobody reserved can hide under it; the
    /// slope test measures at a tight budget for that reason
    /// ([`growth`](crate::search::growth)).
    #[must_use]
    pub fn untracked(&self) -> u64 {
        self.untracked_outside(&[])
    }

    /// [`untracked`](Self::untracked), leaving out the phases whose names start with one
    /// of `exempt`.
    #[must_use]
    pub fn untracked_outside(&self, exempt: &[&str]) -> u64 {
        self.phases
            .iter()
            .filter(|p| !exempt.iter().any(|e| p.0.starts_with(e)))
            .map(|p| p.1)
            .max()
            .unwrap_or(0)
    }
}

fn signed(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

static ONE: Mutex<()> = Mutex::new(());

/// Runs a test's work `f` alone, on a pool of [`THREADS`] threads that have all started.
///
/// Alone: the counters are global, so nothing else may allocate while a run is
/// measured, and the tests of a binary run on several threads under `cargo test`.
/// Started: a thread allocates its state (its queue, its thread-local data) when it
/// first runs, and a measurement during which threads start counts that against the
/// operation, more or less of it as the threads are fast or slow to start.
///
/// # Panics
///
/// If the pool cannot be made.
pub fn alone<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    let _one = ONE.lock().unwrap_or_else(PoisonError::into_inner);
    par::with_threads(THREADS, || {
        warm_up(THREADS.get());
        f()
    })
    .expect("a pool of threads")
}

/// Lets each of the `threads` threads of the current pool run a task: each task waits
/// until all have begun, or a second has passed.
pub fn warm_up(threads: usize) {
    let begun = AtomicUsize::new(0);
    par::for_each_range(threads, NonZeroUsize::MIN, |_| {
        begun.fetch_add(1, Relaxed);
        let start = Instant::now();
        while begun.load(Relaxed) < threads && start.elapsed() < Duration::from_secs(1) {
            std::thread::yield_now();
        }
    });
}

/// Runs `op` against a [`Ledger`] of `limit` bytes (`None`: unlimited) that refuses its
/// `fail_at`-th reservation, with `scratch` (made empty first) for its files.
///
/// # Panics
///
/// If [`Counting`](counting::Counting) is not the global allocator, or the scratch
/// directory cannot be made.
pub fn measure(
    limit: Option<u64>,
    fail_at: Option<u64>,
    scratch: &Path,
    op: impl FnOnce(&Memory, &Path) -> Result<(), Failure>,
) -> Run {
    assert!(
        counting::installed(),
        "Counting is not the global allocator"
    );
    let _ = std::fs::remove_dir_all(scratch);
    std::fs::create_dir_all(scratch).expect("the scratch directory");
    let ledger = Arc::new(Ledger::new(limit, Pages::HEAP, fail_at));
    let memory = Memory::from(Arc::clone(&ledger));
    let base = counting::start();
    let result = catch_unwind(AssertUnwindSafe(|| op(&memory, scratch)));
    let heap_peak = counting::peak() - base;
    let heap_left = i64::try_from(counting::live()).unwrap_or(i64::MAX) - signed(base as u64);
    let phases = counting::untracked_by_phase();
    let outcome = match result {
        Ok(Ok(())) => Outcome::Done,
        Ok(Err(Failure::Budget(e))) => Outcome::Refused(e),
        Ok(Err(Failure::Other(e))) => Outcome::Failed(e),
        Err(panic) => Outcome::Panicked(
            panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default(),
        ),
    };
    let scratch_left = std::fs::read_dir(scratch).map_or(0, Iterator::count);
    Run {
        outcome,
        heap_peak: heap_peak as u64,
        ledger_peak: ledger.budget().peak(),
        heap_left,
        ledger_left: ledger.budget().used(),
        reservations: ledger.reservations(),
        phases,
        scratch_left,
    }
}

/// The slope between the two largest sizes of `points` (size, bytes, by increasing
/// size), in bytes per unit of size: a growth with the size counts, a growth that
/// stopped below the largest size (a sample of fixed size filling up) does not.
#[must_use]
pub fn top_slope(points: &[(f64, f64)]) -> f64 {
    slope(&points[points.len().saturating_sub(2)..])
}

/// The least-squares slope of `points` (size, bytes): bytes per unit of size.
#[must_use]
pub fn slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |(sx, sy), &(x, y)| (sx + x, sy + y));
    let (mx, my) = (sx / n, sy / n);
    let (sxy, sxx) = points.iter().fold((0.0, 0.0), |(sxy, sxx), &(x, y)| {
        (sxy + (x - mx) * (y - my), sxx + (x - mx) * (x - mx))
    });
    if sxx == 0.0 { 0.0 } else { sxy / sxx }
}

/// The smallest budget with which an operation completes, found by bisection.
#[derive(Clone, Debug)]
pub struct Boundary {
    /// The smallest budget that completes, to within [`step`](Self::step).
    pub smallest: u64,
    /// The ledger's peak without a limit.
    pub unlimited_peak: u64,
    /// Where the bisection stopped: `unlimited_peak / parts`, at least 64 bytes.
    pub step: u64,
}

/// What the planner's declared minimum says against the true one
/// (`m3-02-planner`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Declared {
    /// Declared below the smallest budget that completes: the planner would accept a
    /// budget the run cannot keep, an out-of-memory in waiting.
    TooLow,
    /// Declared at or above it: the ratio declared / true, which above 1 is the share
    /// of budgets refused for nothing.
    Ratio(f64),
}

impl Boundary {
    /// Bisects between 0 and the first of 2, 4, … 64 times the unlimited peak that
    /// completes, to within `1 / parts` of the peak. `run(budget)` says whether the
    /// operation completes with that budget; it must also have checked that a run which
    /// does not complete refused cleanly. A run under a limit can need more than the
    /// unlimited peak, as concurrent reservations race.
    ///
    /// # Panics
    ///
    /// If the operation does not complete at 64 times the unlimited peak.
    pub fn search(unlimited_peak: u64, parts: u64, mut run: impl FnMut(u64) -> bool) -> Self {
        let peak = unlimited_peak.max(1);
        let mut hi = peak * 2;
        while !run(hi) {
            assert!(
                hi < peak * 64,
                "no completion at 64 times the unlimited peak, {peak} B"
            );
            hi *= 2;
        }
        let mut lo = 0_u64;
        let step = (unlimited_peak / parts.max(1)).max(64);
        while hi - lo > step {
            let mid = lo + (hi - lo) / 2;
            if run(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        Self {
            smallest: hi,
            unlimited_peak,
            step,
        }
    }

    /// Compares a planner's declared minimum with the smallest budget found.
    #[must_use]
    pub fn against(&self, declared: u64) -> Declared {
        if declared < self.smallest.saturating_sub(self.step) {
            Declared::TooLow
        } else {
            Declared::Ratio(declared as f64 / self.smallest.max(1) as f64)
        }
    }
}

/// Runs `op` once with no failure to count its reservations K, then once for each
/// k = 1 … K with the k-th reservation refused, and returns every run.
pub fn refuse_each(scratch: &Path, op: impl Fn(&Memory, &Path) -> Result<(), Failure>) -> Vec<Run> {
    let first = measure(None, None, scratch, &op);
    (1..=first.reservations)
        .map(|k| measure(None, Some(k), scratch, &op))
        .collect()
}
