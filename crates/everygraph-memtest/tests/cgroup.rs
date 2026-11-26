// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The no-OOM promise as the kernel sees it (issue #53): each operation runs in a child
//! process inside `systemd-run --user --scope -p MemoryMax=budget+slack -p
//! MemorySwapMax=0`, with its buffers from the page backend, as the command line runs
//! it. Each run must complete or refuse, never be killed, and its anonymous memory must
//! stay within the ledger's peak plus a fixed gap. Every record is printed, and appended
//! to the file `EVERYGRAPH_MEMTEST_RECORDS` names when it is set.
//!
//! Skipped where `systemd-run --user` does not work (no systemd user session). The
//! pull-request set is one graph at three budgets; `--ignored` runs the nightly matrix.

#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]
// Byte counts and budgets as fractions of them: positive, far below 2^53.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#![expect(clippy::print_stderr, reason = "the records are the tests' report")]

use std::io::Write as _;
use std::path::Path;

use everygraph_memtest::cgroup::{self, Record};
use everygraph_memtest::graphs::{FAMILIES, Recipe};
use everygraph_memtest::ops::{Input, OPS, Op};

/// What the scope holds above the budget for the process itself: the test binary, its
/// threads' stacks, the input's arrays, and the page cache of the files it maps and
/// writes, which the kernel reclaims before it kills.
const SLACK: u64 = 96 << 20;

/// Most anonymous bytes above the ledger's peak (the "untracked" gap): what the counting
/// allocator allows ([`no_oom.rs`]'s 512 KiB), what the allocator keeps after frees,
/// thread stacks and page rounding.
const GAP: i64 = 32 << 20;

/// The scope of an unlimited run.
const UNLIMITED_SCOPE: u64 = 4 << 30;

/// The test the parent starts again in the child.
const CHILD: &str = "child";

/// Runs what the parent asked for, in a child; does nothing in the parent.
#[test]
fn child() {
    cgroup::child();
}

fn record(op: Op, recipe: &Recipe, budget: Option<u64>, r: &Record) {
    let line = format!(
        "{} {} n={} budget={} outcome={} ledger_peak={} anon_peak={} gap={} memory_peak={} \
         pagetables={} kernel={} shmem={} oom_kill={}",
        op.name(),
        recipe.name.replace(' ', "-"),
        recipe.n,
        budget.map_or_else(|| "unlimited".into(), |b| b.to_string()),
        r.outcome,
        r.ledger_peak,
        r.anon_peak - r.anon_before,
        r.gap(),
        r.memory_peak,
        r.pagetables,
        r.kernel,
        r.shmem,
        r.oom_kill
    );
    eprintln!("{line}");
    if let Some(path) = std::env::var_os("EVERYGRAPH_MEMTEST_RECORDS") {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("the records file");
        writeln!(file, "{line}").expect("a record written");
    }
}

/// Runs `op` on `input` at each budget in `budgets` (fractions of its ledger peak
/// without a limit) and at the absolute `extra` budgets; returns the failures.
fn matrix(op: Op, recipe: &Recipe, input: &Input, fractions: &[f64], extra: &[u64]) -> Vec<String> {
    let mut failures = Vec::new();
    let mut one = |budget: Option<u64>| {
        let scope = budget.map_or(UNLIMITED_SCOPE, |b| b + SLACK + input.heap_bytes());
        match cgroup::run(CHILD, op, &input.dir, budget, scope) {
            Ok(r) => {
                record(op, recipe, budget, &r);
                let clean = matches!(r.outcome.as_str(), "done" | "refused");
                if !clean || r.oom_kill > 0 || r.gap() > GAP {
                    failures.push(format!("{} at {budget:?}: {r:?}", op.name()));
                }
                Some(r)
            }
            Err(e) => {
                failures.push(e);
                None
            }
        }
    };
    let Some(unlimited) = one(None).filter(|_| op.budgeted()) else {
        return failures;
    };
    let peak = unlimited.ledger_peak.max(1);
    for &f in fractions {
        one(Some((peak as f64 * f) as u64));
    }
    for &b in extra {
        one(Some(b));
    }
    failures
}

#[test]
fn the_process_stays_within_its_budget() {
    if !cgroup::available() {
        eprintln!("skipped: systemd-run --user --scope does not work here");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("everygraph-memtest-cgroup-{}", std::process::id()));
    let recipe = FAMILIES[0].at(1 << 14);
    let input = Input::prepare(&recipe, &dir);
    let failures: Vec<String> = OPS
        .into_iter()
        .flat_map(|op| matrix(op, &recipe, &input, &[0.5, 1.0, 2.0], &[]))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The nightly matrix: larger graphs of two families, budgets across each operation's
/// peak and the fixed budgets of issue #53 from 0.5 to 8 GiB.
#[test]
#[ignore = "an hour, gigabytes: cargo test --release -p everygraph-memtest --test cgroup -- --ignored"]
fn the_process_stays_within_its_budget_nightly() {
    if !cgroup::available() {
        eprintln!("skipped: systemd-run --user --scope does not work here");
        return;
    }
    const GIB: u64 = 1 << 30;
    let fixed = [GIB / 2, GIB, 2 * GIB, 4 * GIB, 8 * GIB];
    let mut failures = Vec::new();
    for recipe in [
        FAMILIES[0].at(1 << 22),
        FAMILIES[1].at(1 << 21),
        FAMILIES[6].at(1 << 20),
    ] {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("everygraph-memtest-cgroup-{}", std::process::id()));
        let input = Input::prepare(&recipe, &dir);
        for op in OPS {
            failures.extend(matrix(
                op,
                &recipe,
                &input,
                &[0.25, 0.5, 0.9, 1.0, 1.1, 2.0],
                &fixed,
            ));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
