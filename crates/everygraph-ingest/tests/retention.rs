// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the process holds above the ledger, as the input grows. A dictionary build at a
//! fixed budget is run at three sizes; at each, once the build has returned, the
//! anonymous memory of the process is some bytes above what it held before plus what
//! the ledger still holds (the dictionary). That excess must not grow with the number
//! of keys: an allocator that keeps freed blocks holds more of them the longer the
//! build runs, until the process exceeds the budget it was promised
//! (`design/DECISIONS.md`, "The memory promise covers the process";
//! `docs/research/sota-testing.md`, challenge 3). With the build's large buffers from
//! `everygraph_io::PAGES`, they are given back when freed.
//!
//! The excess is read after the build, not at its peak. The ledger counts a block when
//! it is reserved and the kernel counts a page when it is first written, so the two
//! peaks fall at different moments. In exploratory runs (PR #NNN, with the kernel's
//! high-water mark `VmHWM`) the anonymous peak was 1.5–7.7 MB below the ledger's, by an
//! amount that changed from run to run and was largest at the smallest build.
//!
//! What it catches: buffers that go back to glibc's heap instead of the page backend,
//! at these sizes. What it does not: the choice of the backend's threshold, whose effect
//! shows at hundreds of millions of keys (the benchmark's `build-500m`), and memory held
//! during the build but given back before it returns.
//!
//! Each build runs in a child process, this test binary again, so that no allocator
//! state carries from one size to the next. The control test runs the same builds on
//! the heap, where glibc's retention grows; it depends on glibc, so it is ignored by
//! default: `cargo test -p everygraph-ingest --test retention -- --ignored`.

#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]

mod support;

use std::num::NonZeroUsize;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use everygraph_core::{Budget, Pages};
use everygraph_ingest::ids::{BuildOptions, ColumnType, build_dictionary};
use support::{Files, Synthetic, url};

/// The budget of every build.
const BUDGET: u64 = 32 << 20;
/// Distinct keys of the three builds.
const SIZES: [u64; 3] = [250_000, 500_000, 1_000_000];
/// Threads of every build: retention is kept per allocator arena, about one per thread.
const THREADS: usize = 8;
/// Most bytes the excess may grow from the smallest build to the largest. In
/// exploratory runs of this test (a loaded machine, four cores, PR #NNN) it grew by
/// 18–52 MB on glibc's heap in 5 runs and by 3 MB or less with the page backend in 50.
const GROWTH_LIMIT: i64 = 8 << 20;

/// Set in a child: how many keys to build, and where the buffers come from.
const KEYS_VAR: &str = "EVERYGRAPH_RETENTION_KEYS";
const PAGES_VAR: &str = "EVERYGRAPH_RETENTION_PAGES";

#[test]
fn the_excess_above_the_ledger_does_not_grow_with_the_input() {
    let excess = excess_by_size("maps");
    let growth = excess[SIZES.len() - 1] - excess[0];
    assert!(
        growth <= GROWTH_LIMIT,
        "with the page backend the excess above the ledger grew by {growth} B from {} to {} keys: {excess:?}",
        SIZES[0],
        SIZES[SIZES.len() - 1]
    );
}

/// The measurement sees retention: on glibc's heap the excess grows.
#[test]
#[ignore = "depends on glibc's allocator: cargo test -p everygraph-ingest --test retention -- --ignored"]
fn on_the_heap_the_excess_grows() {
    let excess = excess_by_size("heap");
    let growth = excess[SIZES.len() - 1] - excess[0];
    assert!(
        growth > GROWTH_LIMIT,
        "on the heap the excess grew by only {growth} B: {excess:?}"
    );
}

/// The excess above the ledger after the build, in bytes, at each of [`SIZES`], each in
/// its own process.
fn excess_by_size(pages: &str) -> Vec<i64> {
    let exe = std::env::current_exe().unwrap();
    let excess: Vec<i64> = SIZES
        .iter()
        .map(|n| {
            let out = Command::new(&exe)
                .args(["--exact", "one_build", "--nocapture", "--test-threads", "1"])
                .env(KEYS_VAR, n.to_string())
                .env(PAGES_VAR, pages)
                .env("RAYON_NUM_THREADS", THREADS.to_string())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success(),
                "the build of {n} keys failed: {stdout}{}",
                String::from_utf8_lossy(&out.stderr)
            );
            stdout
                .lines()
                .find_map(|l| Some(l.split_once("excess ")?.1))
                .unwrap_or_else(|| panic!("no result from the child: {stdout}"))
                .parse()
                .unwrap()
        })
        .collect();
    #[expect(
        clippy::print_stderr,
        reason = "the measurements are the test's report"
    )]
    {
        eprintln!("{pages}: excess above the ledger by size {SIZES:?}: {excess:?} B");
    }
    excess
}

/// One build, in a child: prints the anonymous memory above what the process held
/// before, minus what the ledger holds, once the build has returned. Does nothing in
/// the parent.
#[test]
fn one_build() {
    let (Ok(n), Ok(pages)) = (std::env::var(KEYS_VAR), std::env::var(PAGES_VAR)) else {
        return;
    };
    let pages = match pages.as_str() {
        "maps" => everygraph_io::PAGES,
        _ => Pages::HEAP,
    };
    let n: u64 = n.parse().unwrap();
    warm_up();
    let budget = Budget::new(BUDGET).with_pages(pages);
    let scratch = Files::new(
        std::env::temp_dir().join(format!("everygraph-retention-{}", std::process::id())),
    );
    let source = Synthetic {
        column: ColumnType::Utf8,
        n,
        stream: n + n / 2,
        make: url,
    };
    let baseline = rss_anon();
    let built = build_dictionary(&source, &BuildOptions::default(), &budget, &scratch).unwrap();
    assert!(!built.dictionary.is_empty());
    let above = i64::try_from(rss_anon()).unwrap() - i64::try_from(baseline).unwrap();
    let excess = above - i64::try_from(budget.used()).unwrap();
    #[expect(clippy::print_stdout, reason = "the parent reads the result here")]
    {
        println!("excess {excess}");
    }
}

/// Lets every thread of the pool run a task before the baseline is read, so that their
/// stacks and allocator arenas are in it: each task waits until all have begun, or a
/// second has passed.
fn warm_up() {
    let begun = AtomicUsize::new(0);
    everygraph_core::par::for_each_range(THREADS, NonZeroUsize::MIN, |_| {
        begun.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();
        while begun.load(Ordering::Relaxed) < THREADS && start.elapsed() < Duration::from_secs(1) {
            std::thread::yield_now();
        }
    });
}

/// The anonymous memory the process holds, in bytes: `Anonymous` of
/// `/proc/self/smaps_rollup`, counted from the page tables when it is read. `RssAnon`
/// of `/proc/self/status` is summed from per-thread or per-CPU counters that are
/// updated in batches.
fn rss_anon() -> u64 {
    let status = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap();
    let kib: u64 = status
        .lines()
        .find_map(|l| l.strip_prefix("Anonymous:"))
        .and_then(|v| v.trim().strip_suffix("kB"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    kib << 10
}
