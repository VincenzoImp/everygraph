// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Work counters of the frontier driver and of the decoder, for experiment #266
//! (`lab/m3-27-frontier-counters`). Built only with the feature `frontier-counters`.
//! The counters are global: run one kernel at a time, then [`take`] them.
//!
//! A worker adds to thread-local counts; a driver task adds them to the global ones when
//! it ends. That is correct because no body that counts runs a nested parallel loop.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, PoisonError};

/// One call of [`Plan::frontier_with`](super::Plan::frontier_with).
#[derive(Clone, Copy, Debug, Default)]
pub struct Call {
    /// What the kernel said it was doing ([`tag`]).
    pub tag: &'static str,
    /// The kernel's key for the call: a round, a level, a step.
    pub key: u64,
    /// Nodes of the frontier.
    pub nodes: u64,
    /// Runs of `FRONTIER_GRAIN` nodes. One run is done on the calling thread.
    pub runs: u64,
    /// Entries handed to the body, the side lists of `Lists::get` included.
    pub entries: u64,
    /// Entries of the largest run.
    pub max_run: u64,
    /// The longest list of the frontier itself.
    pub max_list: u64,
    /// The most side entries read by one call of the body: a chain that one task does.
    pub max_chain: u64,
    /// Side lists read through `Lists::get`.
    pub side_lists: u64,
    /// Counter-based draws in the bodies.
    pub draws: u64,
}

/// The global counts after a step of a kernel ([`mark`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct Mark {
    /// What the step was.
    pub tag: &'static str,
    /// The kernel's key for the step.
    pub key: u64,
    /// Entries the codec decoded so far.
    pub decoded: u64,
    /// Entries the drivers handed to bodies so far.
    pub handed: u64,
    /// Entries the bodies read so far: up to the last entry they used.
    pub examined: u64,
}

/// Everything counted since the last [`take`].
#[derive(Debug, Default)]
pub struct Counts {
    /// Every call of the frontier driver, in order.
    pub calls: Vec<Call>,
    /// Every mark, in order.
    pub marks: Vec<Mark>,
    /// Restarts of k-core without sampling.
    pub restarts: u64,
}

static TAG: Mutex<(&str, u64)> = Mutex::new(("", 0));
static COUNTS: Mutex<Counts> = Mutex::new(Counts {
    calls: Vec::new(),
    marks: Vec::new(),
    restarts: 0,
});
static DECODED: AtomicU64 = AtomicU64::new(0);
static HANDED: AtomicU64 = AtomicU64::new(0);
static EXAMINED: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Draws, decoded entries, examined entries of this worker, not yet added.
    static LOCAL: Cell<[u64; 3]> = const { Cell::new([0; 3]) };
}

fn local(i: usize, n: u64) {
    LOCAL.with(|c| {
        let mut x = c.get();
        x[i] += n;
        c.set(x);
    });
}

/// Names the calls of the frontier driver that follow.
pub fn tag(tag: &'static str, key: u64) {
    *TAG.lock().unwrap_or_else(PoisonError::into_inner) = (tag, key);
}

/// Records the global counts now, under `tag`.
pub fn mark(tag: &'static str, key: u64) {
    let m = Mark {
        tag,
        key,
        decoded: DECODED.load(Relaxed),
        handed: HANDED.load(Relaxed),
        examined: EXAMINED.load(Relaxed),
    };
    lock().marks.push(m);
}

/// Everything counted so far; the counters start again from zero.
pub fn take() -> Counts {
    for c in [&DECODED, &HANDED, &EXAMINED] {
        c.store(0, Relaxed);
    }
    std::mem::take(&mut *lock())
}

fn lock() -> std::sync::MutexGuard<'static, Counts> {
    COUNTS.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn draw() {
    local(0, 1);
}

pub(crate) fn decoded(n: usize) {
    local(1, n as u64);
}

pub(crate) fn examined(n: usize) {
    local(2, n as u64);
}

pub(crate) fn handed(n: u64) {
    HANDED.fetch_add(n, Relaxed);
}

pub(crate) fn restart() {
    lock().restarts += 1;
}

/// Adds this worker's counts to the global ones; returns its draws.
pub(crate) fn flush() -> u64 {
    let [draws, decoded, examined] = LOCAL.with(|c| c.replace([0; 3]));
    DECODED.fetch_add(decoded, Relaxed);
    EXAMINED.fetch_add(examined, Relaxed);
    draws
}

/// One run of a frontier call.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Run {
    pub entries: u64,
    pub max_list: u64,
    pub max_chain: u64,
    pub side_lists: u64,
    pub draws: u64,
}

/// Records a frontier call of `nodes` nodes from its runs.
pub(crate) fn call(nodes: usize, runs: &[Run]) {
    let (tag, key) = *TAG.lock().unwrap_or_else(PoisonError::into_inner);
    let mut c = Call {
        tag,
        key,
        nodes: nodes as u64,
        runs: runs.len() as u64,
        ..Call::default()
    };
    for r in runs {
        c.entries += r.entries;
        c.max_run = c.max_run.max(r.entries);
        c.max_list = c.max_list.max(r.max_list);
        c.max_chain = c.max_chain.max(r.max_chain);
        c.side_lists += r.side_lists;
        c.draws += r.draws;
    }
    lock().calls.push(c);
}
