---
title: Harness: record every in-process iteration and name the measurement level
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P2, risk:low, size:S
---
**Why.** {{#m3-18-harness-statistics}} gave the report rank intervals and `compare` ratio intervals, but over one level only: the processes the harness starts. Inside each process `glab run --reps 2` reports the best of 2 iterations, so `headline`'s `seconds` is a median over processes of a minimum over iterations. That mixed statistic is stated only in a spec comment and in the `level` condition (`in-process kernel, best of 2 (tc: 1)`). Best-of misleads (Georges et al., OOPSLA'07), and the levels should be named and repeated at the top (Kalibera and Jones, ISMM'13). The A/A k-core 16 T case (27% spread, `docs/research/sota-measurement.md` § 3.1) may be partly the best-of-2. This was the fourth scope item of {{#m3-18-harness-statistics}}; it moved here because it changes the JSON output of the lab and product runners, not only the harness.

**Scope**
- [ ] Runners print every in-process iteration time as an array (`glab run`, the kernel bench binary, the comparison runners)
- [ ] Specs read the array with `reduce = "median"` and also record the minimum as a second metric; drop `--reps 2 → best` from the commands
- [ ] The report names the level of every metric: per process, or reduced over iterations and how
- [ ] Re-run `headline` exploratory: medians and interval widths against the current best-of-2 rows, labelled exploratory

**Acceptance**
- No shipped spec reports a best-of statistic without saying so in its metric name or unit
- The report states the level of every number it prints

**Depends on** {{#m3-18-harness-statistics}}
