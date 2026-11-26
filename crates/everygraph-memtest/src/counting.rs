// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`Counting`], a global allocator that passes every call to `System` and counts the
//! bytes the program asks for: the bytes in use, their peak, and, per phase, the most
//! bytes in use beyond what the ledger holds. A test binary installs it with
//! `#[global_allocator] static HEAP: Counting = Counting;`.
//!
//! The counts are requested sizes, so they do not depend on the allocator underneath;
//! what an allocator keeps after a free is measured by the control group
//! ([`cgroup`](crate::cgroup)).
//!
//! A phase is the purpose of the last reservation made on the thread that allocates, or,
//! on a thread that made none since [`start`], the last purpose reserved on any thread
//! (the map of maxima of `PerfFuzz`, `docs/research/sota-testing.md` challenge 3, item 3).
//!
//! Safe code cannot replace the global allocator, hence the module's `unsafe`
//! (CONTRIBUTING.md, "`unsafe`").

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Phases told apart; later purposes share the last one.
const PHASES: usize = 64;

/// The name of phase 0: no reservation made yet.
const NONE: &str = "(before any reservation)";

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
/// Anonymous bytes the ledger of the measured run holds now.
static LEDGER: AtomicUsize = AtomicUsize::new(0);
/// Incremented by [`start`], so that the phase a thread kept from an earlier run is
/// not used.
static GENERATION: AtomicUsize = AtomicUsize::new(0);
/// The phase of the last reservation on any thread.
static LAST: AtomicUsize = AtomicUsize::new(0);
static NAMES: [OnceLock<&'static str>; PHASES] = [const { OnceLock::new() }; PHASES];
static CLAIMED: AtomicUsize = AtomicUsize::new(1);
static UNTRACKED: [AtomicUsize; PHASES] = [const { AtomicUsize::new(0) }; PHASES];

thread_local! {
    /// (generation, phase) of the last reservation on this thread.
    static TAG: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// Bytes in use now.
pub fn live() -> usize {
    LIVE.load(Relaxed)
}

/// The most bytes in use since [`start`].
pub fn peak() -> usize {
    PEAK.load(Relaxed)
}

/// Whether [`Counting`] is the global allocator of this program.
pub fn installed() -> bool {
    // Nothing else changes the counter, and a test harness has allocated by now.
    std::hint::black_box(Box::new(0_u8));
    live() > 0
}

/// Starts a measurement: the peak and the per-phase maxima restart from the bytes in
/// use now, which are returned.
pub fn start() -> usize {
    let now = live();
    BASE.store(now, Relaxed);
    PEAK.store(now, Relaxed);
    LEDGER.store(0, Relaxed);
    LAST.store(0, Relaxed);
    GENERATION.fetch_add(1, Relaxed);
    for max in &UNTRACKED {
        max.store(0, Relaxed);
    }
    now
}

/// The most bytes in use above the start and beyond the ledger, by phase, for each
/// phase where it was above zero.
pub fn untracked_by_phase() -> Vec<(&'static str, u64)> {
    // Read before the vector below is allocated, which would count.
    let maxima: [usize; PHASES] = std::array::from_fn(|i| UNTRACKED[i].load(Relaxed));
    (0..PHASES)
        .filter_map(|i| {
            let max = maxima[i];
            let name = if i == 0 { Some(&NONE) } else { NAMES[i].get() };
            Some((*name?, u64::try_from(max).ok()?)).filter(|&(_, b)| b > 0)
        })
        .collect()
}

/// Records the ledger's anonymous bytes in use, after a reservation or a release.
pub(crate) fn ledger(bytes: u64) {
    LEDGER.store(usize::try_from(bytes).unwrap_or(usize::MAX), Relaxed);
}

/// Makes `purpose` the phase of this thread, and the last phase.
pub(crate) fn enter(purpose: &'static str) {
    let phase = phase_of(purpose);
    LAST.store(phase, Relaxed);
    let generation = GENERATION.load(Relaxed);
    let _ = TAG.try_with(|tag| tag.set((generation, phase)));
}

/// The phase that `purpose` names, claimed if new. Allocates nothing: this runs inside
/// reservations, which may be made while the allocator counts.
fn phase_of(purpose: &'static str) -> usize {
    let claimed = CLAIMED.load(Relaxed).min(PHASES);
    if let Some(i) = (1..claimed).find(|&i| NAMES[i].get() == Some(&purpose)) {
        return i;
    }
    let i = CLAIMED.fetch_add(1, Relaxed);
    if i >= PHASES {
        return PHASES - 1;
    }
    let _ = NAMES[i].set(purpose);
    i
}

fn grew(bytes: usize) {
    let now = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(now, Relaxed);
    let beyond = now
        .saturating_sub(BASE.load(Relaxed))
        .saturating_sub(LEDGER.load(Relaxed));
    if beyond > 0 {
        let generation = GENERATION.load(Relaxed);
        let phase = TAG
            .try_with(Cell::get)
            .ok()
            .filter(|&(g, _)| g == generation)
            .map_or_else(|| LAST.load(Relaxed), |(_, p)| p);
        UNTRACKED[phase].fetch_max(beyond, Relaxed);
    }
}

/// The counting allocator (module documentation).
#[derive(Debug)]
pub struct Counting;

// SAFETY: every block comes from `System` and goes back to it with the layout it was
// allocated with; the counters do not touch the blocks, and counting allocates nothing.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc` is passed on unchanged.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as for `alloc`.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        // SAFETY: `ptr` was allocated by `System` with `layout` (every block of this
        // allocator is).
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // The old and the new block may both exist while the data moves.
        grew(new_size);
        // SAFETY: `ptr` was allocated by `System` with `layout`, and the caller
        // guarantees that `new_size` is valid for it.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if p.is_null() {
            LIVE.fetch_sub(new_size, Relaxed);
        } else {
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
        p
    }
}
