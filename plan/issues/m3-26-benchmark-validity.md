---
title: Comparisons that hold: validated outputs, equal measurement, intervals, true out-of-core
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P0, risk:medium, size:M
---
**Why.** The state-of-the-art review of 2026-11-26 (`/home/tux/graphlab/sota-2026-11-26/synthesis.md` § 1 rows 9, 10, 12; § 8; `benchmarks.md`) found that no speed comparison with the best codes holds today:
- The reference runs (#55, PR #250) do not check that the tools computed the same result.
- The tools are measured differently: best of 5 against one run; f64 against int32 SSSP; f32 GVE-Leiden; `-march=native` for one tool only; no thread pinning.
- Ratios between tools on a loaded machine change sign: WCC, GAP ÷ ours on lj, went from 2.53 to 0.83 between rounds (`lab/m3-12-reference-runs/results/rounds.jsonl` on the PR #250 branch).
- The `friendster-ooc` case in `bench/specs/kernels.toml` runs warm under a 2 GiB limit with nothing evicted first, so it may not be out of core.

**Scope**
- [ ] Every reference run's output is checked against ours: component count, core histogram, distances or parents, PageRank within tolerance, modularity. A run whose output differs is not reported.
- [ ] Equal measurement: the same repetitions and statistic, the same numeric precision (or both precisions shown), the same compiler flags, thread pinning stated, and the same storage (heap or mmap) stated.
- [ ] Ratios between tools come only from official idle-machine runs with 95% intervals (#194 statistics, #195 interleaving). Exploratory cross-tool ratios are never quoted.
- [ ] A spec lint: any case whose memory limit is below the graph's store size must be cold (evicted first) and must record `read_bytes`, `pgmajfault` and refaults from `memory.stat`. Fix `friendster-ooc`.
- [ ] Add ConnectIt, Parallel-KCore and PASGAL to the reference set, plus a road graph and a web graph. Add McSherry's single-thread COST code as a line.

**Progress.** PR #250 does the first four parts for `bench/specs/references.toml`: `bench/runners/run.py validate` checks every tool's output against Everygraph's before the timed cases (a failed check is not reported; GBBS PageRank and igraph PageRank fail by construction and are marked unavailable); every tool runs its kernel 5 times in one process and reports the median (searches: the mean over GAP's 64 sources), SSSP in f64 and in integers, GVE-Leiden in f32 and f64, every C++ tool and Everygraph without `-march=native`, every process pinned to the 16 physical cores; the out-of-core lint in `everygraph-bench check` and `pgmajfault` and `refaults` in every row; `friendster-ooc` cold; ConnectIt, Parallel-KCore, PASGAL, the COST line, roadnet-ca and web-google added. Left: the official orkut window (F4), whose intervals need {{#m3-18-harness-statistics}} and {{#m3-19-harness-interleave}}; a `-march=native` variant of every tool next to the generic one (built by `bench/references/build.sh native`, not run); an unpinned variant; a large web graph (uk-2005) and a large road graph (europe-osm, DIMACS USA), which are not in the registry.

**Done when.** One official orkut window gives validated outputs and ratios with intervals for every reference tool (the review's experiment F4), and the lint passes on every spec.

**Depends on** {{#m3-12-perf-tracking}}, {{#m3-18-harness-statistics}}, {{#m3-19-harness-interleave}}
