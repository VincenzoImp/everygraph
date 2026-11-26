// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `everygraph-bench report`: markdown tables and chart-ready JSON from result rows.
//!
//! Every number comes with its conditions — graph and the spec's other conditions,
//! threads, memory limit, cache state, and whether the run was idle-checked — and with
//! the count of repetitions behind it. Summaries use completed, undisturbed rows only;
//! the others are counted in the "excluded" column, never silently dropped.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{Value, json};

use super::extract::median;
use super::row::{Row, Status};
use super::spec::{BUILTIN_METRICS, PLAN_QUANTITIES};
use super::stats::{self, CONFIDENCE, Interval};
use super::units::human_size;

/// A value with its unit, for people: seconds to three or four digits, bytes in
/// GiB/MiB, anything else to four significant digits.
#[must_use]
pub fn format_value(v: f64, unit: Option<&str>) -> String {
    match unit {
        Some("s") if v.abs() >= 1.0 => format!("{v:.2} s"),
        Some("s") if v.abs() >= 0.01 => format!("{v:.3} s"),
        Some("s") => format!("{:.2} ms", v * 1000.0),
        Some("B") if v >= 0.0 => {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "non-negative byte count"
            )]
            let b = v.round() as u64;
            human_size(b)
        }
        Some(u) => format!("{} {u}", significant(v)),
        None => significant(v),
    }
}

fn significant(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v}");
    }
    let magnitude = v.abs().log10().floor();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small digit count"
    )]
    let decimals = (3.0 - magnitude).clamp(0.0, 9.0) as usize;
    format!("{v:.decimals$}")
}

/// Statistics of one metric over the counted rows of a case.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    /// Median.
    pub median: f64,
    /// Smallest.
    pub min: f64,
    /// Largest.
    pub max: f64,
    /// The rank interval of the median ([`stats::median_interval`]).
    pub interval: Interval,
    /// The values, in row order.
    pub values: Vec<f64>,
}

impl Stats {
    /// `None` for no values.
    #[must_use]
    pub fn of(values: Vec<f64>) -> Option<Self> {
        let interval = stats::median_interval(&values, CONFIDENCE)?;
        Some(Self {
            median: median(&values),
            min: values.iter().copied().fold(f64::INFINITY, f64::min),
            max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            interval,
            values,
        })
    }

    /// `median [low, high]`; the confidence follows when it is not the case's own (a
    /// metric that some rows lack).
    fn cell(&self, unit: Option<&str>, counted: usize) -> String {
        if self.values.len() == 1 {
            return format_value(self.median, unit);
        }
        let mut cell = format!(
            "{} [{}, {}]",
            format_value(self.median, unit),
            format_value(self.interval.low, unit),
            format_value(self.interval.high, unit)
        );
        if self.values.len() != counted {
            let _ = write!(cell, " {:.1}%", self.interval.confidence * 100.0);
        }
        cell
    }
}

/// The confidence of the median intervals of `n` values, for people.
fn confidence_label(n: usize) -> String {
    if n < 2 {
        return "–".into();
    }
    let c = stats::interval_confidence(n, CONFIDENCE);
    if c >= CONFIDENCE {
        format!("{:.1}%", c * 100.0)
    } else {
        format!("{:.1}%, no 95% interval", c * 100.0)
    }
}

/// The rows of one case in one run.
#[derive(Debug, Clone)]
pub struct CaseRows<'a> {
    /// All its rows.
    pub rows: Vec<&'a Row>,
}

impl<'a> CaseRows<'a> {
    /// The first row.
    #[must_use]
    pub fn first(&self) -> &'a Row {
        self.rows[0]
    }

    /// The rows that count: completed and undisturbed.
    #[must_use]
    pub fn counted(&self) -> Vec<&'a Row> {
        self.rows.iter().copied().filter(|r| r.counts()).collect()
    }

    /// Statistics of a metric over the counted rows.
    #[must_use]
    pub fn stats(&self, metric: &str) -> Option<Stats> {
        Stats::of(
            self.counted()
                .iter()
                .filter_map(|r| r.metric(metric))
                .collect(),
        )
    }

    fn excluded(&self) -> String {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &self.rows {
            if r.status != Status::Ok {
                *counts.entry(r.status.as_str()).or_default() += 1;
            } else if r.disturbance.disturbed {
                *counts.entry("disturbed").or_default() += 1;
            }
        }
        if counts.is_empty() {
            return "–".into();
        }
        counts
            .iter()
            .map(|(k, n)| format!("{n} {k}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The conditions, for people.
    #[must_use]
    pub fn conditions(&self) -> String {
        let r = self.first();
        let mut parts: Vec<String> = r
            .conditions
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        if let Some(d) = &r.dataset {
            if !r.conditions.contains_key("graph") {
                parts.insert(0, format!("data {}", d.name));
            }
        }
        if let Some(runner) = &r.runner {
            parts.push(format!("runner {runner}"));
        }
        parts.join(", ")
    }
}

/// One run: its rows, grouped by report table and case, in file order.
#[derive(Debug, Clone)]
pub struct RunRows<'a> {
    /// The run id.
    pub run_id: String,
    /// Groups in order; each a list of cases in order.
    pub groups: Vec<(String, Vec<CaseRows<'a>>)>,
}

impl<'a> RunRows<'a> {
    /// Any row of the run.
    #[must_use]
    pub fn any(&self) -> &'a Row {
        self.groups[0].1[0].first()
    }

    /// The cases, in order.
    pub fn cases(&self) -> impl Iterator<Item = &CaseRows<'a>> {
        self.groups.iter().flat_map(|(_, c)| c.iter())
    }
}

/// Groups rows by run, then by group, then by case, keeping the order of first appearance.
#[must_use]
pub fn group(rows: &[Row]) -> Vec<RunRows<'_>> {
    let mut runs: Vec<RunRows<'_>> = Vec::new();
    for r in rows {
        let run = runs
            .iter()
            .position(|x| x.run_id == r.run_id)
            .unwrap_or_else(|| {
                runs.push(RunRows {
                    run_id: r.run_id.clone(),
                    groups: Vec::new(),
                });
                runs.len() - 1
            });
        let groups = &mut runs[run].groups;
        let g = groups
            .iter()
            .position(|(g, _)| *g == r.group)
            .unwrap_or_else(|| {
                groups.push((r.group.clone(), Vec::new()));
                groups.len() - 1
            });
        let cases = &mut groups[g].1;
        match cases.iter_mut().find(|c| c.first().case == r.case) {
            Some(c) => c.rows.push(r),
            None => cases.push(CaseRows { rows: vec![r] }),
        }
    }
    runs
}

fn metric_columns(cases: &[CaseRows<'_>]) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    let mut add = |m: &str| {
        if !cols.iter().any(|c| c == m) {
            cols.push(m.to_string());
        }
    };
    for c in cases {
        add(&c.first().primary);
    }
    for c in cases {
        for m in c.rows.iter().flat_map(|r| r.metrics.keys()) {
            add(m);
        }
    }
    for m in ["wall_seconds", "memory_peak_bytes", "anon_peak_bytes"] {
        add(m);
    }
    if cases
        .iter()
        .any(|c| c.rows.iter().any(|r| r.stop.is_some()))
    {
        add("stop_seconds");
    }
    cols
}

fn unit_of(cases: &[CaseRows<'_>], metric: &str) -> Option<String> {
    BUILTIN_METRICS
        .iter()
        .find(|(n, _)| *n == metric)
        .map(|(_, u)| (*u).to_string())
        .or_else(|| {
            cases
                .iter()
                .find_map(|c| c.first().units.get(metric).cloned())
        })
}

/// The outcome of one reference check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// Within the tolerance, or under the bound.
    Within,
    /// Outside it.
    Off,
    /// No counted repetition to compare: never a pass.
    Unchecked,
}

/// Counts of reference checks over some runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Checks within their tolerance or bound.
    pub within: usize,
    /// Checks outside.
    pub off: usize,
    /// References with no counted repetition to compare.
    pub unchecked: usize,
}

impl Verdict {
    /// Every reference was checked and met.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.off == 0 && self.unchecked == 0
    }

    /// `k within, j off, m unchecked`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{} within, {} off, {} unchecked",
            self.within, self.off, self.unchecked
        )
    }
}

/// The reference checks of all cases of `rows`.
#[must_use]
pub fn verdict(rows: &[Row]) -> Verdict {
    let mut v = Verdict::default();
    for run in group(rows) {
        for c in run.cases() {
            for (_, check) in reference_checks(c) {
                match check {
                    Check::Within => v.within += 1,
                    Check::Off => v.off += 1,
                    Check::Unchecked => v.unchecked += 1,
                }
            }
        }
    }
    v
}

/// The reference checks of one case: text and outcome per reference.
#[must_use]
pub fn reference_checks(case: &CaseRows<'_>) -> Vec<(String, Check)> {
    let first = case.first();
    first
        .references
        .iter()
        .map(|r| {
            let unit = first.unit(&r.metric);
            let Some(s) = case.stats(&r.metric) else {
                let what = r.value.map_or_else(
                    || {
                        format!(
                            "at most {}",
                            format_value(r.max.unwrap_or(f64::INFINITY), unit)
                        )
                    },
                    |v| format_value(v, unit),
                );
                return (
                    format!("{what} ({}): UNCHECKED, no counted repetition", r.source),
                    Check::Unchecked,
                );
            };
            if let Some(v) = r.value {
                let diff = (s.median - v) / v;
                let ok = diff.abs() <= r.tolerance;
                (
                    format!(
                        "{} ({}): {:+.1}%, {}",
                        format_value(v, unit),
                        r.source,
                        diff * 100.0,
                        if ok {
                            format!("within {:.0}%", r.tolerance * 100.0)
                        } else {
                            format!("OFF by more than {:.0}%", r.tolerance * 100.0)
                        }
                    ),
                    if ok { Check::Within } else { Check::Off },
                )
            } else {
                let bound = r.max.unwrap_or(f64::INFINITY);
                let ok = s.max <= bound;
                (
                    format!(
                        "at most {} ({}): largest {}, {}",
                        format_value(bound, unit),
                        r.source,
                        format_value(s.max, unit),
                        if ok { "met" } else { "MISSED" }
                    ),
                    if ok { Check::Within } else { Check::Off },
                )
            }
        })
        .collect()
}

/// Replaces the references the rows ran with by the current ones of `spec`, case by
/// case (rows of cases the spec no longer has keep theirs). For references set after a
/// run, from that run's own measurements. Returns how many rows were changed.
pub fn use_spec_references(rows: &mut [Row], spec: &super::spec::Spec) -> usize {
    let mut changed = 0;
    for r in rows.iter_mut() {
        if let Some(c) = spec.cases.iter().find(|c| c.id == r.case) {
            let refs: Vec<super::row::ReferenceRecord> = c
                .references
                .iter()
                .map(|x| super::row::ReferenceRecord {
                    metric: x.metric.clone(),
                    value: x.value,
                    max: x.max,
                    tolerance: x.tolerance,
                    source: x.source.clone(),
                })
                .collect();
            if refs != r.references {
                r.references = refs;
                changed += 1;
            }
        }
    }
    changed
}

fn run_header(out: &mut String, run: &RunRows<'_>, target: f64) {
    let r = run.any();
    let rows: usize = run.cases().map(|c| c.rows.len()).sum();
    let disturbed: usize = run
        .cases()
        .flat_map(|c| &c.rows)
        .filter(|r| r.disturbance.disturbed)
        .count();
    let _ = writeln!(out, "## {} — run {}\n", r.spec, run.run_id);
    let _ = writeln!(
        out,
        "- {}",
        if r.official {
            "**Official run**: lock held, idle gate passed at the start (1-minute load and MemAvailable); each repetition checked for other activity, and disturbed ones excluded from the numbers below."
        } else {
            "**Exploratory run**: the machine was not checked for idleness. Do not quote these numbers as results (CLAUDE.md § 3)."
        }
    );
    let _ = writeln!(out, "- Machine: {}", r.machine.summary());
    let _ = writeln!(
        out,
        "- Revision {}{}; spec SHA-256 {}…",
        r.git.short(),
        if r.git.dirty {
            " with uncommitted changes"
        } else {
            ""
        },
        &r.spec_sha256[..r.spec_sha256.len().min(12)]
    );
    if let Some(seed) = r.seed {
        let _ = writeln!(
            out,
            "- Interleaved: rounds over the cases in a seeded random order, reversed in every second round (seed {seed}); `round` and `position` are in every row."
        );
    }
    let _ = writeln!(
        out,
        "- {rows} rows; {disturbed} disturbed (excluded). Cells: median [low, high] over the counted repetitions, an exact rank interval of the median (Le Boudec 2010, Th. 2.1; Hoefler and Belli, SC'15) whose confidence is the `confidence` column; it is 95% or more from 6 counted repetitions. `n for ±{:.0}%`: estimated repetitions for a 95% interval within ±{:.0}% of the median of the primary metric (normal approximation, spread from the median absolute deviation).\n",
        target * 100.0,
        target * 100.0
    );
}

/// The default relative half-width of the interval `n for ±x%` aims at.
pub const TARGET_HALF_WIDTH: f64 = 0.02;

/// The markdown report of `rows` (one or more runs); `target` is the relative
/// half-width the "repetitions needed" column aims at.
#[must_use]
pub fn markdown(rows: &[Row], target: f64) -> String {
    let mut out = String::new();
    for run in group(rows) {
        run_header(&mut out, &run, target);
        let mut unavailable = Vec::new();
        for (name, cases) in &run.groups {
            let measured: Vec<CaseRows<'_>> = cases
                .iter()
                .filter(|c| c.rows.iter().any(|r| r.status != Status::Unavailable))
                .cloned()
                .collect();
            for c in cases
                .iter()
                .filter(|c| c.rows.iter().all(|r| r.status == Status::Unavailable))
            {
                unavailable.push(c.first());
            }
            if measured.is_empty() {
                continue;
            }
            let cols = metric_columns(&measured);
            let has_refs = measured.iter().any(|c| !c.first().references.is_empty());
            let _ = writeln!(out, "### {name}\n");
            let mut header = vec![
                "case",
                "conditions",
                "threads",
                "memory limit",
                "cache",
                "n",
                "confidence",
                "excluded",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
            header.push(format!("n for ±{:.0}%", target * 100.0));
            // Cells carry their units.
            header.extend(cols.iter().cloned());
            if has_refs {
                header.push("reference".into());
            }
            let _ = writeln!(out, "| {} |", header.join(" | "));
            let _ = writeln!(out, "|{}", "---|".repeat(header.len()));
            for c in &measured {
                let r = c.first();
                let counted = c.counted().len();
                let mut cells = vec![
                    format!("`{}`", r.case),
                    c.conditions(),
                    r.threads.to_string(),
                    human_size(r.memory_limit_bytes),
                    format!("{:?}", r.cache.mode).to_lowercase(),
                    format!("{counted} of {}", c.rows.len()),
                    confidence_label(counted),
                    c.excluded(),
                    c.stats(&r.primary)
                        .and_then(|s| stats::repetitions_for(&s.values, target))
                        .map_or_else(|| "–".into(), |n| n.to_string()),
                ];
                for m in &cols {
                    let unit = unit_of(&measured, m);
                    cells.push(
                        c.stats(m)
                            .map_or_else(|| "–".into(), |s| s.cell(unit.as_deref(), counted)),
                    );
                }
                if has_refs {
                    let checks = reference_checks(c);
                    cells.push(if checks.is_empty() {
                        "–".into()
                    } else {
                        checks
                            .into_iter()
                            .map(|(t, _)| t)
                            .collect::<Vec<_>>()
                            .join("; ")
                    });
                }
                let _ = writeln!(out, "| {} |", cells.join(" | "));
            }
            let _ = writeln!(out);
        }
        if !unavailable.is_empty() {
            let _ = writeln!(out, "### Not run\n");
            for r in unavailable {
                let _ = writeln!(
                    out,
                    "- `{}`: {}",
                    r.case,
                    r.error.as_deref().unwrap_or("unavailable")
                );
            }
            let _ = writeln!(out);
        }
        let failures: Vec<&Row> = run
            .cases()
            .flat_map(|c| c.rows.iter().copied())
            .filter(|r| !matches!(r.status, Status::Ok | Status::Unavailable))
            .collect();
        if !failures.is_empty() {
            let _ = writeln!(out, "### Rows that did not complete\n");
            for r in failures {
                let mut e = r.error.clone().unwrap_or_default();
                if e.len() > 300 {
                    let mut cut = 300;
                    while !e.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    e.truncate(cut);
                    e.push('…');
                }
                let which = r.warmup.map_or_else(
                    || format!("rep {}", r.repetition),
                    |w| format!("warm-up {w}"),
                );
                let _ = writeln!(
                    out,
                    "- `{}` {which}: {}: {}",
                    r.case,
                    r.status.as_str(),
                    e.replace('\n', " ")
                );
            }
            let _ = writeln!(out);
        }
        coverage_section(&mut out, &run);
        let run_rows: Vec<Row> = run
            .cases()
            .flat_map(|c| c.rows.iter().map(|r| (*r).clone()))
            .collect();
        let v = verdict(&run_rows);
        if v.within + v.off + v.unchecked > 0 {
            let _ = writeln!(
                out,
                "**References: {}.** A reference with no counted repetition (all disturbed or failed) is unchecked, never a pass.\n",
                v.line()
            );
        }
    }
    out
}

/// Per quantity: rows with an estimate, how many actuals lay in `[low, high]`, and the
/// median relative error (ux-contract P-7, P-9). Wall time counts on official,
/// undisturbed rows only (P-7: its coverage is defined on an idle machine).
#[must_use]
pub fn coverage(run: &RunRows<'_>) -> Vec<(String, usize, usize, Option<f64>)> {
    PLAN_QUANTITIES
        .iter()
        .filter_map(|q| {
            let errs: Vec<_> = run
                .cases()
                .flat_map(|c| c.rows.iter())
                .filter(|r| r.status == Status::Ok)
                .filter(|r| *q != "seconds" || (r.official && !r.disturbance.disturbed))
                .filter_map(|r| r.estimate_error.as_ref()?.get(*q).copied())
                .collect();
            if errs.is_empty() {
                return None;
            }
            let inside = errs.iter().filter(|e| e.inside).count();
            let rel: Vec<f64> = errs.iter().filter_map(|e| e.relative).collect();
            let med = if rel.is_empty() {
                None
            } else {
                Some(median(&rel))
            };
            Some(((*q).to_string(), errs.len(), inside, med))
        })
        .collect()
}

fn coverage_section(out: &mut String, run: &RunRows<'_>) {
    let rows = coverage(run);
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "### Estimates against actuals (ux-contract P-7, P-9)\n"
    );
    let _ = writeln!(
        out,
        "| quantity | rows | actual in [low, high] | share | median relative error |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|");
    for (q, n, inside, med) in rows {
        #[allow(clippy::cast_precision_loss, reason = "row counts")]
        let share = inside as f64 / n as f64 * 100.0;
        let med = med.map_or_else(|| "undefined".into(), |m| format!("{:+.1}%", m * 100.0));
        let _ = writeln!(out, "| {q} | {n} | {inside} | {share:.0}% | {med} |");
    }
    let _ = writeln!(
        out,
        "\nWall time (`seconds`) counts on official, undisturbed rows only. A relative error is undefined when the actual is 0 and the estimate is not; such a row is outside its interval.\n"
    );
}

/// Chart-ready JSON: one series per case and metric, with its conditions, values and
/// median interval.
#[must_use]
pub fn chart_json(rows: &[Row], target: f64) -> Value {
    let mut runs = Vec::new();
    let mut series = Vec::new();
    for run in group(rows) {
        let r = run.any();
        runs.push(json!({
            "run_id": run.run_id,
            "spec": r.spec,
            "official": r.official,
            "machine": r.machine,
            "git": r.git,
            "coverage": coverage(&run).into_iter().map(|(q, n, inside, med)| json!({
                "quantity": q, "rows": n, "inside": inside, "median_relative_error": med,
            })).collect::<Vec<_>>(),
        }));
        for (_, cases) in &run.groups {
            let cols = metric_columns(cases);
            for c in cases {
                let f = c.first();
                if f.status == Status::Unavailable {
                    continue;
                }
                for m in &cols {
                    let Some(s) = c.stats(m) else { continue };
                    series.push(json!({
                        "run_id": run.run_id,
                        "case": f.case,
                        "benchmark": f.benchmark,
                        "group": f.group,
                        "vars": f.vars,
                        "conditions": f.conditions,
                        "dataset": f.dataset.as_ref().map(|d| &d.name),
                        "threads": f.threads,
                        "memory_limit_bytes": f.memory_limit_bytes,
                        "cache": f.cache.mode,
                        "official": f.official,
                        "metric": m,
                        "unit": unit_of(cases, m),
                        "primary": *m == f.primary,
                        "n": s.values.len(),
                        "rows": c.rows.len(),
                        "disturbed": c.rows.iter().filter(|r| r.disturbance.disturbed).count(),
                        "median": s.median,
                        "interval_low": s.interval.low,
                        "interval_high": s.interval.high,
                        "confidence": s.interval.confidence,
                        "repetitions_for_target": stats::repetitions_for(&s.values, target),
                        "target_half_width": target,
                        "min": s.min,
                        "max": s.max,
                        "values": s.values,
                    }));
                }
            }
        }
    }
    json!({ "schema": 1, "runs": runs, "series": series })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::row::tests::{busy, sample_row};
    use crate::harness::row::{EstimateValue, ReferenceRecord, estimate_errors};

    fn run_of(values: &[f64]) -> Vec<Row> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| sample_row("pr32", u32::try_from(i).expect("small") + 1, *v))
            .collect()
    }

    #[test]
    fn medians_exclude_disturbed_and_failed_rows_and_say_so() {
        let mut rows = run_of(&[0.40, 0.38, 0.39, 0.90, 0.10]);
        rows[3].disturbance = busy();
        rows[4].status = Status::Oom;
        rows[4].error = Some("the OOM killer ended 1 process".into());
        rows[0].references.push(ReferenceRecord {
            metric: "seconds".into(),
            value: Some(0.38),
            max: None,
            tolerance: 0.10,
            source: "FINDINGS".into(),
        });
        let md = markdown(&rows, 0.02);
        assert!(md.contains("**Official run**"), "{md}");
        // 3 counted: [min, max] is a 75% interval (1 - 2 / 2^3); 22 repetitions for ±2%.
        assert!(md.contains("| `pr32` | graph orkut, precision f32 | 32 | 8.0 GiB | warm | 3 of 5 | 75.0%, no 95% interval | 1 disturbed, 1 oom | 22 |"), "{md}");
        assert!(md.contains("0.390 s [0.380 s, 0.400 s]"), "{md}");
        assert!(md.contains("0.380 s (FINDINGS): +2.6%, within 10%"), "{md}");
        assert!(md.contains("1 disturbed (excluded)"), "{md}");
        assert!(md.contains("`pr32` rep 5: oom: the OOM killer"), "{md}");

        let chart = chart_json(&rows, 0.02);
        let s = &chart["series"][0];
        assert_eq!(s["metric"], "seconds");
        assert_eq!(s["n"], 3);
        assert_eq!(s["rows"], 5);
        assert_eq!(s["disturbed"], 1);
        assert_eq!(s["median"], 0.39);
        assert_eq!(
            (s["interval_low"].as_f64(), s["interval_high"].as_f64()),
            (Some(0.38), Some(0.40))
        );
        assert_eq!(s["confidence"], 0.75);
        assert_eq!(s["repetitions_for_target"], 22);
        assert_eq!(s["conditions"]["graph"], "orkut");
        assert_eq!(s["threads"], 32);
        assert_eq!(s["memory_limit_bytes"], 8u64 << 30);
    }

    #[test]
    fn exploratory_runs_are_labelled_and_unavailable_cases_listed() {
        let mut rows = run_of(&[1.0]);
        rows[0].official = false;
        let mut skipped = sample_row("duckpgq", 0, 0.0);
        skipped.status = Status::Unavailable;
        skipped.error = Some("needs duckdb 1.3".into());
        skipped.group = "engines".into();
        rows.push(skipped);
        let md = markdown(&rows, 0.02);
        assert!(md.contains("**Exploratory run**"));
        assert!(
            md.contains("### Not run\n\n- `duckpgq`: needs duckdb 1.3"),
            "{md}"
        );
        assert!(!md.contains("### engines"), "{md}");
    }

    #[test]
    fn a_bound_reference_checks_the_largest_value() {
        let mut rows = run_of(&[1.2, 2.4]);
        for r in &mut rows {
            r.references.push(ReferenceRecord {
                metric: "seconds".into(),
                value: None,
                max: Some(2.0),
                tolerance: 0.1,
                source: "ux-contract C-4".into(),
            });
        }
        let runs = group(&rows);
        let checks = reference_checks(&runs[0].groups[0].1[0]);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].1, Check::Off);
        assert!(
            checks[0].0.contains("largest 2.40 s, MISSED"),
            "{}",
            checks[0].0
        );
    }

    #[test]
    fn coverage_of_estimates_with_synthetic_rows() {
        let est = |v: f64| EstimateValue {
            value: v,
            low: v * 0.8,
            high: v * 1.2,
            basis: "model".into(),
        };
        let mut rows = Vec::new();
        for (i, (estimated, actual)) in [(10.0, 10.5), (10.0, 13.0), (10.0, 9.0), (10.0, 0.0)]
            .into_iter()
            .enumerate()
        {
            let mut r = sample_row("ingest", u32::try_from(i).expect("small") + 1, 1.0);
            let estimate: BTreeMap<String, EstimateValue> = [
                ("seconds".to_string(), est(estimated)),
                ("output_bytes".to_string(), est(estimated)),
            ]
            .into_iter()
            .collect();
            let actual: BTreeMap<String, f64> = [
                ("seconds".to_string(), actual),
                ("output_bytes".to_string(), actual),
            ]
            .into_iter()
            .collect();
            r.estimate_error = Some(estimate_errors(&estimate, &actual));
            r.estimate = Some(estimate);
            r.actual = Some(actual);
            rows.push(r);
        }
        // A disturbed row does not count for wall time, and an exploratory run's wall
        // time never counts.
        rows[2].disturbance = busy();
        let runs = group(&rows);
        let cov = coverage(&runs[0]);
        let get = |q: &str| cov.iter().find(|c| c.0 == q).cloned().expect("present");
        // seconds: rows 0, 1, 3 count (2 is disturbed); inside: 10.5 only.
        assert_eq!(get("seconds").1, 3);
        assert_eq!(get("seconds").2, 1);
        // output_bytes: all 4 count; inside: 10.5 and 9.0; the zero actual is a miss.
        assert_eq!((get("output_bytes").1, get("output_bytes").2), (4, 2));
        let md = markdown(&rows, 0.02);
        assert!(md.contains("| seconds | 3 | 1 | 33% |"), "{md}");
        assert!(md.contains("| output_bytes | 4 | 2 | 50% |"), "{md}");

        let mut exploratory = rows.clone();
        for r in &mut exploratory {
            r.official = false;
        }
        let runs = group(&exploratory);
        assert!(coverage(&runs[0]).iter().all(|c| c.0 != "seconds"));
    }

    fn cmp(rows: &[Row], max_diff: f64) -> (String, bool) {
        let c = crate::harness::compare::compare(
            rows,
            &crate::harness::compare::CompareOptions {
                max_diff,
                ..Default::default()
            },
        );
        (c.text, c.passed)
    }

    #[test]
    fn two_runs_are_compared_on_the_primary_metric() {
        let a = run_of(&[1.00, 1.02, 0.98]);
        let mut b = run_of(&[1.03, 1.04, 1.02]);
        for r in &mut b {
            r.run_id = "20261126T000000Z-5e4aad8d2f1c".into();
        }
        let rows: Vec<Row> = a.into_iter().chain(b).collect();
        let (md, ok) = cmp(&rows, 0.08);
        assert!(ok, "{md}");
        assert!(
            md.contains("| 1.030 [1.000, 1.061] | 8.0% | within |"),
            "{md}"
        );
        let (md, ok) = cmp(&rows, 0.02);
        assert!(!ok);
        assert!(
            md.contains("OUTSIDE") && md.contains("0 of 1 comparisons within their"),
            "{md}"
        );
    }

    fn with_reference(mut rows: Vec<Row>, value: f64) -> Vec<Row> {
        for r in &mut rows {
            r.references.push(ReferenceRecord {
                metric: "seconds".into(),
                value: Some(value),
                max: None,
                tolerance: 0.10,
                source: "test".into(),
            });
        }
        rows
    }

    #[test]
    fn the_verdict_counts_unchecked_cases_as_not_passed() {
        let within = with_reference(run_of(&[1.0, 1.01]), 1.0);
        let mut off = with_reference(run_of(&[2.0, 2.1]), 1.0);
        let mut unchecked = with_reference(run_of(&[1.0, 1.0]), 1.0);
        for r in &mut off {
            r.case = "slow".into();
        }
        for r in &mut unchecked {
            r.case = "all-disturbed".into();
            r.disturbance = busy();
        }
        let v = verdict(&within);
        assert!(v.passed());
        assert_eq!(v.line(), "1 within, 0 off, 0 unchecked");
        let rows: Vec<Row> = within.into_iter().chain(off).chain(unchecked).collect();
        let v = verdict(&rows);
        assert_eq!(
            v,
            Verdict {
                within: 1,
                off: 1,
                unchecked: 1
            }
        );
        assert!(!v.passed());
        let md = markdown(&rows, 0.02);
        assert!(
            md.contains("**References: 1 within, 1 off, 1 unchecked.**"),
            "{md}"
        );
        assert!(md.contains("UNCHECKED, no counted repetition"), "{md}");
    }

    #[test]
    fn compare_needs_official_runs_of_one_spec_on_one_machine_and_two_repetitions() {
        let second = |rows: Vec<Row>| -> Vec<Row> {
            rows.into_iter()
                .map(|mut r| {
                    r.run_id = "20261126T000000Z-5e4aad8d2f1c".into();
                    r
                })
                .collect()
        };
        let a = run_of(&[1.0, 1.0]);
        let ok: Vec<Row> = a
            .iter()
            .cloned()
            .chain(second(run_of(&[1.01, 1.0])))
            .collect();
        assert!(cmp(&ok, 0.05).1);

        let mut b = second(run_of(&[1.0, 1.0]));
        b.iter_mut().for_each(|r| r.official = false);
        let (md, pass) = cmp(&a.iter().cloned().chain(b).collect::<Vec<_>>(), 0.05);
        assert!(!pass && md.contains("exploratory"), "{md}");

        let mut b = second(run_of(&[1.0, 1.0]));
        b.iter_mut().for_each(|r| r.spec_sha256 = "f".repeat(64));
        let (md, pass) = cmp(&a.iter().cloned().chain(b).collect::<Vec<_>>(), 0.05);
        assert!(!pass && md.contains("another version of the spec"), "{md}");

        let mut b = second(run_of(&[1.0, 1.0]));
        b.iter_mut()
            .for_each(|r| r.machine.id = "ffffffffffffffff".into());
        let (md, pass) = cmp(&a.iter().cloned().chain(b).collect::<Vec<_>>(), 0.05);
        assert!(!pass && md.contains("another machine"), "{md}");

        // One counted repetition is not enough.
        let mut b = second(run_of(&[1.0, 1.0]));
        b[1].status = Status::Failed;
        let (md, pass) = cmp(&a.into_iter().chain(b).collect::<Vec<_>>(), 0.05);
        assert!(!pass && md.contains("fewer than 2 counted"), "{md}");
    }

    #[test]
    fn references_can_come_from_the_current_spec() {
        // The rows ran with a reference of 1.0 s; the spec now says 2.0 s.
        let mut rows = with_reference(run_of(&[2.0, 2.02]), 1.0);
        assert_eq!(verdict(&rows).off, 1);
        let spec = crate::harness::spec::Spec::parse(
            r#"
schema = 1
name = "demo"
description = "d"
[defaults]
memory_limit = "1G"
[[benchmark]]
name = "pr32"
command = ["x"]
metric = [{ name = "seconds", json = "/s", unit = "s" }]
reference = [{ metric = "seconds", value = 2.0, source = "measured" }]
"#,
        )
        .expect("spec");
        assert_eq!(use_spec_references(&mut rows, &spec), 2);
        assert_eq!(
            verdict(&rows),
            Verdict {
                within: 1,
                off: 0,
                unchecked: 0
            }
        );
        assert_eq!(rows[0].references[0].source, "measured");
        // Applying it again changes nothing.
        assert_eq!(use_spec_references(&mut rows, &spec), 0);
    }

    #[test]
    fn values_are_formatted_with_units() {
        assert_eq!(format_value(0.0134, Some("s")), "0.013 s");
        assert_eq!(format_value(0.0021, Some("s")), "2.10 ms");
        assert_eq!(format_value(3.571, Some("s")), "3.57 s");
        assert_eq!(format_value(1_610_612_736.0, Some("B")), "1.5 GiB");
        assert_eq!(format_value(93.21, Some("Medges/s")), "93.21 Medges/s");
        assert_eq!(format_value(12345.0, None), "12345");
    }
}
