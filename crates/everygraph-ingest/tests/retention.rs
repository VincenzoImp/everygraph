// SPDX-License-Identifier: Apache-2.0 OR MIT

//! What the process holds above the ledger, as the input grows. A dictionary build at a
//! fixed budget is run at three sizes; at each, the anonymous memory of the process
//! (`RssAnon`, sampled every millisecond) peaks some bytes above the ledger's peak, or
//! none. Those bytes must not grow with the number of keys: an allocator that keeps freed
//! blocks holds more of them the longer the build runs, until the process exceeds the
//! budget it was promised (`design/DECISIONS.md`, "The memory promise covers the
//! process"; `docs/research/sota-testing.md`, challenge 3). With the build's large
//! buffers from `everygraph_io::PAGES`, they are given back when freed.
//!
//! Each build runs in a child process, this test binary again, so that no allocator
//! state carries from one size to the next. The control test runs the same builds on
//! the heap, where glibc's retention grows; it depends on glibc, so it is ignored by
//! default: `cargo test -p everygraph-ingest --test retention -- --ignored`.

#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]

mod support;

use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use everygraph_core::{Budget, Pages};
use everygraph_ingest::ids::{BuildOptions, ColumnType, build_dictionary};
use support::{Files, Synthetic, url};

/// The budget of every build.
const BUDGET: u64 = 32 << 20;
/// Distinct keys of the three builds.
const SIZES: [u64; 3] = [250_000, 500_000, 1_000_000];
/// Threads of every build: retention is kept per allocator arena, about one per thread.
const THREADS: &str = "8";
/// Most bytes the excess above the ledger may grow from the smallest build to the
/// largest. In exploratory runs (release build, 8 threads, a loaded machine) it grew by
/// 24–40 MiB on glibc's heap and by at most 1 MiB with the page backend.
const GROWTH_LIMIT: u64 = 8 << 20;

/// Set in a child: how many keys to build, and where the buffers come from.
const KEYS_VAR: &str = "EVERYGRAPH_RETENTION_KEYS";
const PAGES_VAR: &str = "EVERYGRAPH_RETENTION_PAGES";

#[test]
fn the_excess_above_the_ledger_does_not_grow_with_the_input() {
    let excess = excess_by_size("maps");
    let growth = excess[SIZES.len() - 1].saturating_sub(excess[0]);
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
    let growth = excess[SIZES.len() - 1].saturating_sub(excess[0]);
    assert!(
        growth > GROWTH_LIMIT,
        "on the heap the excess grew by only {growth} B: {excess:?}"
    );
}

/// The excess above the ledger, in bytes, at each of [`SIZES`], each in its own process.
fn excess_by_size(pages: &str) -> Vec<u64> {
    let exe = std::env::current_exe().unwrap();
    let excess: Vec<u64> = SIZES
        .iter()
        .map(|n| {
            let out = Command::new(&exe)
                .args(["--exact", "one_build", "--nocapture", "--test-threads", "1"])
                .env(KEYS_VAR, n.to_string())
                .env(PAGES_VAR, pages)
                .env("RAYON_NUM_THREADS", THREADS)
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

/// One build, in a child: prints the peak of the anonymous memory above what the
/// process held before, minus the ledger's peak (0 if below it). Does nothing in the
/// parent.
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
    // Threads exist before the baseline is read.
    everygraph_core::par::for_each_range(64, std::num::NonZeroUsize::MIN, |_| {});
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
    let peak = AtomicU64::new(baseline);
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                peak.fetch_max(rss_anon(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        // The sampler stops when the build ends, also by a panic.
        struct Stop<'d>(&'d AtomicBool);
        impl Drop for Stop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let stop = Stop(&done);
        let built = build_dictionary(&source, &BuildOptions::default(), &budget, &scratch);
        drop(stop);
        assert!(!built.unwrap().dictionary.is_empty());
    });
    scratch.remove();
    let excess = (peak.into_inner() - baseline).saturating_sub(budget.peak());
    #[expect(clippy::print_stdout, reason = "the parent reads the result here")]
    {
        println!("excess {excess}");
    }
}

/// The anonymous memory the process holds, in bytes (`RssAnon` of `/proc/self/status`).
fn rss_anon() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let kib: u64 = status
        .lines()
        .find_map(|l| l.strip_prefix("RssAnon:"))
        .and_then(|v| v.trim().strip_suffix("kB"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    kib << 10
}
