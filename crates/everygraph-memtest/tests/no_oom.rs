// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The no-OOM promise in process (issue #53, `docs/research/sota-testing.md` challenge
//! 3): for every operation, under the counting allocator,
//!
//! - the heap held beyond the ledger does not grow with the graph (a slope test over
//!   sizes, on every family of generated graphs, and over the graphs a search finds);
//! - at every budget, swept across the smallest one that completes, a run completes
//!   within its budget or ends in a typed refusal that gives everything back;
//! - with the k-th reservation refused, for every k, the same.
//!
//! The sizes here keep the pull-request run to about two minutes; `--ignored` runs the
//! nightly sizes and a longer search.

#![forbid(unsafe_code)]
// Byte counts and budgets as fractions of them: positive, far below 2^53.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#![expect(
    clippy::print_stderr,
    reason = "the measured values are the tests' report"
)]

use std::path::{Path, PathBuf};

use everygraph_memtest::counting::Counting;
use everygraph_memtest::graphs::{FAMILIES, Recipe};
use everygraph_memtest::ops::{Input, Kernel, OPS, Op, Regime, known};
use everygraph_memtest::probe::{
    Boundary, Declared, Outcome, Run, alone, measure, refuse_each, slope,
};
use everygraph_memtest::search::{climb, growth};

#[global_allocator]
static HEAP: Counting = Counting;

/// Nodes of the slope test: they straddle the edge-map's chunks (2^16 entries) and the
/// smallest reduction blocks (64 nodes up to 2^18 nodes).
const SIZES: [u32; 3] = [1 << 12, 1 << 14, 1 << 16];

/// Most untracked bytes per node. One `u32` per node, the lab's WCC state that ran out of
/// memory (FINDINGS § B1), is 4; what is left today is below 0.6 (O(max degree) and
/// O(n / 256) buffers).
const SLOPE_LIMIT: f64 = 1.0;

/// Most untracked bytes of any run: the dictionary's key sample (about 250 KB, before
/// its first reservation) is the largest.
const UNTRACKED_LIMIT: u64 = 512 << 10;

/// Most heap a refusal below the declared minimum may allocate: a refusal before any
/// work allocates nothing that grows with the input.
const REFUSAL_LIMIT: u64 = 64 << 10;

/// Most heap a finished run may leave above its start.
const LEFT_LIMIT: i64 = 64 << 10;

/// A directory of its own, removed on drop, on the build directory's disk: `/tmp` may be
/// held in memory (tmpfs), where scratch files are memory too.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("everygraph-memtest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The slope test of `ops` on `recipes` at `sizes`, each run at a tight budget; returns
/// the failures. A known finding's phases are left out of the fit, and must still grow on
/// the first family, so that the entry goes with the fix.
fn slopes(ops: &[Op], recipes: &[Recipe], sizes: &[u32], dir: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    for recipe in recipes {
        for (&op, g) in ops.iter().zip(growth(ops, recipe, sizes, dir)) {
            let s = slope(&g.points);
            let largest = g.points.iter().map(|p| p.1).fold(0.0, f64::max);
            eprintln!(
                "{:>18} {:>20}: {s:7.3} B/node, untracked {:?} B at budgets {:?}; at the largest size by phase {:?}",
                recipe.name,
                op.name(),
                g.points.iter().map(|p| p.1 as u64).collect::<Vec<_>>(),
                g.budgets,
                g.phases
            );
            if s > SLOPE_LIMIT || largest > UNTRACKED_LIMIT as f64 {
                failures.push(format!(
                    "{} on {}: {s:.3} B/node, {largest} B at most",
                    op.name(),
                    recipe.name
                ));
            }
            let exempt = slope(&g.exempt);
            if let Some(k) = known(op).filter(|_| recipe.name == FAMILIES[0].name) {
                eprintln!(
                    "{:>18} {:>20}: known, {exempt:.3} B/node in {:?}",
                    recipe.name,
                    op.name(),
                    k.phases
                );
                if exempt <= SLOPE_LIMIT {
                    failures.push(format!(
                        "{} no longer holds untracked heap that grows in {:?} ({exempt:.3} B/node): \
                         remove it from `ops::KNOWN` ({})",
                        op.name(),
                        k.phases,
                        k.why
                    ));
                }
            }
        }
    }
    failures
}

#[test]
fn untracked_heap_does_not_grow_with_the_graph() {
    alone(|| {
        let dir = Dir::new("slope");
        let failures = slopes(&OPS, &FAMILIES, &SIZES, &dir.0);
        assert!(failures.is_empty(), "{failures:#?}");
    });
}

/// The lab's WCC (FINDINGS § B1) kept one `u32` per node outside any budget. The same
/// state next to the production WCC must fail the slope test, at about 4 bytes per node.
#[test]
fn the_slope_test_catches_the_lab_wcc_state() {
    alone(|| {
        let dir = Dir::new("canary");
        let wcc = Op::Kernel(Kernel::Wcc, Regime::Mapped);
        let mut with = Vec::new();
        let mut without = Vec::new();
        for n in SIZES {
            let input = Input::prepare(&FAMILIES[0].at(n), &dir.0.join("input"));
            let canary = measure(None, None, &dir.0.join("scratch"), |memory, scratch| {
                let lab_state = std::hint::black_box(vec![0_u32; input.nodes as usize]);
                let done = wcc.run(&input, memory, scratch);
                drop(lab_state);
                done
            });
            let clean = measure(None, None, &dir.0.join("scratch"), |memory, scratch| {
                wcc.run(&input, memory, scratch)
            });
            with.push((f64::from(input.nodes), canary.untracked() as f64));
            without.push((f64::from(input.nodes), clean.untracked() as f64));
        }
        let (canary, clean) = (slope(&with), slope(&without));
        eprintln!("WCC with the lab's state: {canary:.3} B/node; without: {clean:.3} B/node");
        assert!(
            (3.5..=8.0).contains(&canary),
            "the canary shows {canary:.3} B/node"
        );
        assert!(clean <= SLOPE_LIMIT, "WCC alone shows {clean:.3} B/node");
    });
}

/// What a run at a budget may end in: a completion within the budget, or a refusal that
/// gave back every reservation, every file and the heap.
fn check(op: Op, budget: Option<u64>, run: &Run) -> bool {
    let at = format!("{} at budget {budget:?}: {run:?}", op.name());
    assert!(
        run.ledger_left == 0 && run.scratch_left == 0 && run.heap_left <= LEFT_LIMIT,
        "left something behind: {at}"
    );
    let exempt = known(op).map_or(&[][..], |k| k.phases);
    assert!(
        run.untracked_outside(exempt) <= UNTRACKED_LIMIT,
        "untracked heap: {at}"
    );
    match &run.outcome {
        Outcome::Done => {
            let limit = budget.unwrap_or(u64::MAX);
            assert!(run.ledger_peak <= limit, "ledger above the budget: {at}");
            true
        }
        Outcome::Refused { .. } => false,
        Outcome::Failed(_) | Outcome::Panicked(_) => panic!("not a typed refusal: {at}"),
    }
}

#[test]
fn every_budget_completes_or_refuses_cleanly() {
    alone(|| {
        let dir = Dir::new("boundary");
        for recipe in [FAMILIES[0], FAMILIES[1]] {
            let input = Input::prepare(&recipe.at(1 << 12), &dir.0.join("input"));
            for op in OPS.into_iter().filter(|op| op.budgeted()) {
                budgets_across_the_boundary(op, &input, &dir.0.join("scratch"), recipe.name);
            }
        }
    });
}

fn budgets_across_the_boundary(op: Op, input: &Input, scratch: &Path, graph: &str) {
    let run = |budget: Option<u64>| {
        let r = measure(budget, None, scratch, |memory, scratch| {
            op.run(input, memory, scratch)
        });
        (check(op, budget, &r), r)
    };
    let (done, unlimited) = run(None);
    assert!(done);
    let boundary = Boundary::search(unlimited.ledger_peak, 256, |b| run(Some(b)).0);
    // Near the boundary, which of two concurrent reservations is refused depends on the
    // schedule, so a run can complete at a budget and refuse at a larger one; both are
    // clean, and `check` has seen to that.
    let sweep: Vec<(f64, bool)> = [0.5, 0.9, 0.99, 1.0, 1.01, 1.1, 2.0]
        .into_iter()
        .map(|f| {
            let budget = (boundary.smallest as f64 * f) as u64;
            (f, run(Some(budget)).0)
        })
        .collect();
    eprintln!(
        "{graph:>10} {:>20}: smallest budget {} B, ledger peak unlimited {} B; completes at {:?} of it",
        op.name(),
        boundary.smallest,
        boundary.unlimited_peak,
        sweep
            .iter()
            .filter(|s| s.1)
            .map(|s| s.0)
            .collect::<Vec<_>>()
    );
    assert!(
        sweep.last().is_some_and(|s| s.1),
        "{} refuses twice its smallest budget: {sweep:?}",
        op.name()
    );
    if let Some(declared) = op.declared_minimum(input) {
        let verdict = boundary.against(declared);
        eprintln!(
            "{graph:>10} {:>20}: declared {declared} B: {verdict:?}",
            op.name()
        );
        assert_ne!(
            verdict,
            Declared::TooLow,
            "{}: {declared} B declared, below the {} B it needs",
            op.name(),
            boundary.smallest
        );
        let below = measure(Some(declared / 2), None, scratch, |memory, scratch| {
            op.run(input, memory, scratch)
        });
        assert!(
            matches!(below.outcome, Outcome::Refused(_))
                && (below.heap_peak <= REFUSAL_LIMIT || known(op).is_some_and(|k| k.refuses_late)),
            "below the declared minimum: {below:?}"
        );
    }
}

#[test]
fn a_refused_reservation_ends_in_a_typed_error_for_every_k() {
    alone(|| {
        let dir = Dir::new("kth");
        let input = Input::prepare(&FAMILIES[0].at(1 << 10), &dir.0.join("input"));
        for op in OPS.into_iter().filter(|op| op.budgeted()) {
            let runs = refuse_each(&dir.0.join("scratch"), |memory, scratch| {
                op.run(&input, memory, scratch)
            });
            let refused = runs.iter().filter(|r| !check(op, None, r)).count();
            eprintln!(
                "{:>20}: {} reservations, {refused} runs refused",
                op.name(),
                runs.len()
            );
            assert!(!runs.is_empty(), "{} reserves nothing", op.name());
        }
    });
}

#[test]
fn a_search_finds_no_graph_where_untracked_heap_grows() {
    alone(|| {
        let dir = Dir::new("search");
        search(&SIZES, 4, &dir.0);
    });
}

fn search(sizes: &[u32], steps: usize, dir: &Path) {
    let worst = climb(&OPS, &FAMILIES, sizes, steps, 0x5eed, dir);
    let mut failures = Vec::new();
    for (op, w) in &worst {
        let top = w.phases.iter().max_by(|a, b| a.1.0.total_cmp(&b.1.0));
        eprintln!(
            "{op:>20}: worst {:.3} B/node on {:?}; worst phase {top:?}",
            w.slope, w.recipe
        );
        if w.slope > SLOPE_LIMIT {
            failures.push(format!("{op}: {:.3} B/node on {:?}", w.slope, w.recipe));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The nightly sizes: up to 2^18 nodes, where reduction blocks stop growing in number.
#[test]
#[ignore = "minutes: cargo test --release -p everygraph-memtest --test no_oom -- --ignored"]
fn untracked_heap_does_not_grow_at_nightly_sizes() {
    alone(|| {
        let dir = Dir::new("slope-nightly");
        let failures = slopes(&OPS, &FAMILIES, &[1 << 14, 1 << 16, 1 << 18], &dir.0);
        assert!(failures.is_empty(), "{failures:#?}");
    });
}

#[test]
#[ignore = "an hour: cargo test --release -p everygraph-memtest --test no_oom -- --ignored"]
fn a_long_search_finds_no_graph_where_untracked_heap_grows() {
    alone(|| {
        let dir = Dir::new("search-nightly");
        search(&[1 << 12, 1 << 14, 1 << 16], 200, &dir.0);
    });
}
