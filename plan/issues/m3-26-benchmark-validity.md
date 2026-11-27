---
title: Comparisons that hold: validated outputs, equal measurement, intervals, true out-of-core
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P0, risk:medium, size:M
---
**Why.** The state-of-the-art review of 2026-11-26 (`docs/research/sota-review-2026-11.md` § 1 rows 9, 10, 12; § 8; `benchmarks.md`) found that no speed comparison with the best codes holds today:
- The reference runs (#55, PR #250) do not check that the tools computed the same result.
- The tools are measured differently: best of 5 against one run; f64 against int32 SSSP; f32 GVE-Leiden; `-march=native` for one tool only; no thread pinning.
- Ratios between tools on a loaded machine change sign: WCC, GAP ÷ ours on lj, went from 2.53 to 0.83 between rounds (`lab/m3-12-reference-runs/results/rounds.jsonl` at commit 52f1aef of PR #250, removed from the branch later).
- The `friendster-ooc` case in `bench/specs/kernels.toml` runs warm under a 2 GiB limit with nothing evicted first, so it may not be out of core.

**Scope**
- [ ] Every reference run's output is checked against ours: component count, core histogram, distances or parents, PageRank within tolerance, modularity. A run whose output differs is not reported.
- [ ] Equal measurement: the same repetitions and statistic, the same numeric precision (or both precisions shown), the same compiler flags, thread pinning stated, and the same storage (heap or mmap) stated.
- [ ] Ratios between tools come only from official idle-machine runs with 95% intervals (#194 statistics, #195 interleaving). Exploratory cross-tool ratios are never quoted.
- [ ] A spec lint: any case whose memory limit is below the graph's store size must be cold (evicted first) and must record `read_bytes`, `pgmajfault` and refaults from `memory.stat`. Fix `friendster-ooc`.
- [ ] Interleaving checked against run order (moved from {{#m3-19-harness-interleave}}): `kernels-orkut` and `ingest-text` interleaved and consecutive in the same idle window, n = 10 each; medians and interval widths compared and recorded with the result file
- [ ] Add ConnectIt, Parallel-KCore and PASGAL to the reference set, plus a road graph and a web graph. Add McSherry's single-thread COST code as a line.

**Progress.** PR #250 does the first, second, fourth and fifth parts for `bench/specs/references.toml`. `bench/runners/run.py validate` checks every tool's output against Everygraph's before the timed cases, at 16 threads: partitions, core numbers, every distance and every parent, PageRank within L1 1e-6, triangle counts, a modularity floor for Leiden. A case fails on any invalid or skipped check, and `lab/m3-12-reference-runs/tables.py` drops the rows of a tool without a passing validation (`test_tables.py`). Equal measurement: the median of 5 in-process runs, or the mean over GAP's 64 sources; SSSP on costs of the undirected pairs, in f64 and integers; GVE-Leiden in f32 and f64; 2 Leiden iterations where a tool takes a count; no `-march=native`; every process on the 16 physical cores, with the thread binding of each runtime recorded per row. The out-of-core lint (sizes from the registry when the data is absent), `pgmajfault` and `refaults` per row, and `friendster-ooc` cold. ConnectIt, Parallel-KCore, PASGAL, the COST line, roadnet-ca and web-google added. Left: the official orkut window (F4), with the intervals of {{#m3-18-harness-statistics}} and the interleaved rounds of {{#m3-19-harness-interleave}} (both built, PR #284); a `-march=native` variant and an unbound variant (`bench/references/build.sh native` builds the first, no case runs it); a large web graph (uk-2005) and a large road graph (europe-osm, DIMACS USA), which are not in the registry.

**Done when.** One official orkut window gives validated outputs and ratios with intervals for every reference tool (the review's experiment F4), and the lint passes on every spec.

**Depends on** {{#m3-12-perf-tracking}}, {{#m3-18-harness-statistics}}, {{#m3-19-harness-interleave}}
