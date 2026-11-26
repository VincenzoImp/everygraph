---
title: Harness: intervals with their confidence, six repetitions, ratio comparisons
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P1, risk:low, size:S
---
**Why.** The harness ({{#m0-03-bench-harness}}) reports "median (min–max)" of 3 counted repetitions and compares runs by a difference of medians against `--max-diff`; {{#m3-12-perf-tracking}} first planned a flat 7% regression gate (now defined on this issue's comparison). The literature asks for a rank interval of the median at a stated confidence, at least 5–6 repetitions at the top level, and a ratio interval against a per-benchmark threshold calibrated by A/A runs (Hoefler–Belli SC'15; Kalibera–Jones ISMM'13; Barrett et al. OOPSLA'17; Laaber et al. EMSE'19). Our own A/A test, re-analysed offline: the two official idle `headline` runs `bench/results/headline/20261125T101402Z-79634d76b95b.jsonl` and `bench/results/headline/20261125T104322Z-79634d76b95b.jsonl` (same spec and revision, 68 cases × 3 repetitions, all 204 rows undisturbed) differ by ≥ 7% in their medians on 3 of 68 cases (k-core 16 T +14.0%, orkut parse-only +12.1%, Friendster 8 GB −7.5%), ≥ 5% on 4 and ≥ 2% on 13; within-run spread was ≥ 5% in 12 of 68. A 7% gate would fire about three times per comparison with no code change (`docs/research/sota-measurement.md` § 3.1). Decided 2026-11-25 (`docs/research/README.md` M2).

**Scope**
- [ ] The report prints the median with an exact rank interval and its confidence level: with n = 3, [min, max] is exactly a 75% interval of the median (1 − 2·0.5³); with n = 6 it is 96.9%; with n = 10, [x₍₂₎, x₍₉₎] is 97.9% (binomial arithmetic)
- [ ] Official runs default to n = 6; n = 3 stays allowed and is labelled "no 95% interval". One official `headline` window roughly doubles (about 26 → 50 minutes, the report's estimate)
- [ ] `compare` reports the ratio of medians with an interval (bootstrap over rounds, or Fieller) and passes only if the interval lies within ±δ, with δ per case from an A/A run
- [ ] Name the measurement level of every number: record each in-process iteration and let the harness reduce (median over processes, minimum shown); drop `--reps 2 → best` inside the commands, a mixed statistic stated today only in a spec comment
- [ ] Offline first: recompute both runs' intervals above (no machine needed); then one official `headline` run at n = 6 and an A/A `compare` of two runs to set δ per case

**Acceptance**
- The report shows interval and confidence for every case; `compare` gives ratio intervals
- An A/A comparison of two official runs of the same revision passes on every case with the calibrated δ
- {{#m3-12-perf-tracking}}'s regression gate is defined on this comparison

**Pointers.** `docs/research/sota-measurement.md` § 2 (statistics table), § 3.1, § 4 challenges 2 and 10; `crates/everygraph-bench/src/harness/report.rs`.

**Depends on** {{#m0-03-bench-harness}}
