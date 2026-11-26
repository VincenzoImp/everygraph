// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph-bench report --compare`: the primary metric of two cases as a ratio of
//! medians with a 95% interval, against a per-case threshold δ.
//!
//! Across runs, each later run is compared with the first (official runs of one spec on
//! one machine: an A/A run of one revision calibrates δ). Within a run, `--by VAR=BASE`
//! compares the cases with `VAR = BASE` with those that differ from them only in `VAR`:
//! an A/B of two binaries or settings in one interleaved run.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::report::{CaseRows, RunRows, Stats, format_value, group};
use super::row::{Row, Status};
use super::stats::{self, CONFIDENCE};

/// What [`compare`] compares, and against which thresholds.
#[derive(Debug, Clone, Default)]
pub struct CompareOptions {
    /// δ of every case without its own: the ratio interval must lie in
    /// `[1 - δ, 1 + δ]`.
    pub max_diff: f64,
    /// δ by case id, from an A/A calibration ([`Comparison::calibrated`]).
    pub deltas: BTreeMap<String, f64>,
    /// Compare within each run instead of across runs: the cases whose matrix variable
    /// `.0` is `.1` against the cases that differ from them only in that variable.
    pub by: Option<(String, String)>,
}

/// The result of [`compare`].
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    /// The markdown table.
    pub text: String,
    /// Every pair was comparable and its interval lay within its δ.
    pub passed: bool,
    /// For each compared case, the smallest δ its interval meets, rounded up to 0.1%:
    /// from an A/A comparison, the δ to calibrate later comparisons with.
    pub calibrated: BTreeMap<String, f64>,
}

impl Comparison {
    fn refused(text: String) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }
}

/// A case compared with its baseline.
struct Pair<'r, 'a> {
    base: &'r CaseRows<'a>,
    other: &'r CaseRows<'a>,
    against: String,
}

/// The seed of every bootstrap, so that a comparison is reproducible.
const BOOTSTRAP_SEED: u64 = 0x6576_6572_7967_7261;

fn check_runs(runs: &[RunRows<'_>]) -> Option<String> {
    let first = runs[0].any();
    runs.iter().find_map(|run| {
        let r = run.any();
        let problem = if !r.official {
            "is exploratory: only official runs are compared across runs".to_string()
        } else if r.spec_sha256 != first.spec_sha256 {
            format!(
                "ran another version of the spec (SHA-256 {} against {})",
                &r.spec_sha256[..12],
                &first.spec_sha256[..12]
            )
        } else if r.machine.id != first.machine.id {
            format!(
                "ran on another machine ({} against {})",
                r.machine.id, first.machine.id
            )
        } else {
            return None;
        };
        Some(format!("not comparable: run {} {problem}\n", run.run_id))
    })
}

/// The pairs of one run by a matrix variable.
fn pairs_by<'r, 'a>(run: &'r RunRows<'a>, var: &str, base: &str) -> Vec<Pair<'r, 'a>> {
    let cases: Vec<&CaseRows<'a>> = run.cases().collect();
    let mut out = Vec::new();
    for b in cases
        .iter()
        .filter(|c| c.first().vars.get(var).map(String::as_str) == Some(base))
    {
        let fb = b.first();
        for o in &cases {
            let fo = o.first();
            let partner = fo.benchmark == fb.benchmark
                && fo.threads == fb.threads
                && fo.vars.len() == fb.vars.len()
                && fo.vars.get(var).is_some_and(|v| v != base)
                && fo
                    .vars
                    .iter()
                    .all(|(k, v)| k == var || fb.vars.get(k) == Some(v));
            if partner {
                out.push(Pair {
                    base: b,
                    other: o,
                    against: format!("`{}`", fb.case),
                });
            }
        }
    }
    out
}

fn pairs_across<'r, 'a>(runs: &'r [RunRows<'a>]) -> Vec<Pair<'r, 'a>> {
    let mut out = Vec::new();
    for base in runs[0].cases() {
        if base.first().status == Status::Unavailable {
            continue;
        }
        for run in &runs[1..] {
            if let Some(other) = run.cases().find(|c| c.first().case == base.first().case) {
                out.push(Pair {
                    base,
                    other,
                    against: format!("run `{}`", runs[0].run_id),
                });
            }
        }
    }
    out
}

/// Compares the primary metric of cases: across runs, or within each run with
/// [`CompareOptions::by`]. A pair passes when its ratio interval lies within its δ.
#[must_use]
pub fn compare(rows: &[Row], opts: &CompareOptions) -> Comparison {
    let runs = group(rows);
    let (title, pairs) = match &opts.by {
        Some((var, base)) => {
            let pairs: Vec<Pair<'_, '_>> =
                runs.iter().flat_map(|r| pairs_by(r, var, base)).collect();
            if pairs.is_empty() {
                return Comparison::refused(format!(
                    "no case has {var}={base} and a partner that differs from it only in {var}\n"
                ));
            }
            (
                format!("## A/B comparison within each run: {var} against {var}={base}"),
                pairs,
            )
        }
        None => {
            if runs.len() < 2 {
                return Comparison::refused(
                    "compare needs rows of at least two runs, or --by VAR=VALUE\n".into(),
                );
            }
            if let Some(text) = check_runs(&runs) {
                return Comparison::refused(text);
            }
            (
                format!("## Run-to-run comparison against {}", runs[0].run_id),
                pairs_across(&runs),
            )
        }
    };
    let mut out = String::new();
    let _ = writeln!(out, "{title}\n");
    let _ = writeln!(
        out,
        "Primary metric: the ratio of the medians of the counted repetitions, case / baseline, with a 95% percentile-bootstrap interval ({} resamples, each side resampled on its own; Efron and Tibshirani 1993, § 13.3). A pair passes when the interval lies within 1 ± δ.\n",
        stats::RESAMPLES
    );
    if pairs.iter().any(|p| !p.other.first().official) {
        let _ = writeln!(
            out,
            "Exploratory rows: quote the ratios, never the times (CLAUDE.md § 3).\n"
        );
    }
    if opts.by.is_some() && pairs.iter().any(|p| p.other.first().round.is_none()) {
        let _ = writeln!(
            out,
            "Not interleaved (`run --interleave`): the order of the cases can bias these ratios.\n"
        );
    }
    let _ = writeln!(
        out,
        "| case | against | conditions | threads | metric | baseline | median | ratio [95% interval] | δ | verdict |"
    );
    let _ = writeln!(out, "|{}", "---|".repeat(10));
    let (mut within, mut calibrated) = (0, BTreeMap::new());
    for p in &pairs {
        let f = p.other.first();
        let metric = &p.base.first().primary;
        let unit = f.unit(metric);
        let delta = opts.deltas.get(&f.case).copied().unwrap_or(opts.max_diff);
        // At least two counted repetitions on each side, or the median says little.
        let sample = |c: &CaseRows<'_>| c.stats(metric).filter(|s| s.values.len() >= 2);
        let (a, b) = (sample(p.base), sample(p.other));
        let show =
            |s: Option<&Stats>| s.map_or_else(|| "–".into(), |s| format_value(s.median, unit));
        let mut cells = vec![
            format!("`{}`", f.case),
            p.against.clone(),
            p.other.conditions(),
            f.threads.to_string(),
            metric.clone(),
            show(a.as_ref()),
            show(b.as_ref()),
        ];
        let ratio = a.zip(b).and_then(|(a, b)| {
            stats::ratio_interval(&a.values, &b.values, CONFIDENCE, BOOTSTRAP_SEED)
        });
        match ratio {
            Some((r, i)) => {
                let ok = i.low >= 1.0 - delta && i.high <= 1.0 + delta;
                within += usize::from(ok);
                let need = ((1.0 - i.low).max(i.high - 1.0) * 1000.0).ceil() / 1000.0;
                calibrated.insert(f.case.clone(), need.max(0.001));
                cells.push(format!("{r:.3} [{:.3}, {:.3}]", i.low, i.high));
                cells.push(format!("{:.1}%", delta * 100.0));
                cells.push(if ok { "within" } else { "OUTSIDE" }.into());
            }
            None => {
                cells.push("–".into());
                cells.push(format!("{:.1}%", delta * 100.0));
                cells.push("not comparable (fewer than 2 counted repetitions)".into());
            }
        }
        let _ = writeln!(out, "| {} |", cells.join(" | "));
    }
    let _ = writeln!(
        out,
        "\n{within} of {} comparisons within their δ.",
        pairs.len()
    );
    Comparison {
        text: out,
        passed: within == pairs.len() && !pairs.is_empty(),
        calibrated,
    }
}
