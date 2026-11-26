---
title: Performance tracking against reference tools
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P1, risk:low, size:M, lab-evidence
---
**Why.** "Faster than X" claims rot. They need to be regenerated continuously and published with the method.

**Scope**
- [ ] Benchmark specs per kernel on lj, orkut, twitter, Friendster at 1 and 16 threads, three regimes
- [ ] Reference runs with pinned versions: NetworKit, python-igraph, scipy, GVE-Louvain/Leiden, DuckDB (ingest), plus the query engines from the F2 comparison
- [ ] Same-machine runs of the best-known kernel codes, built under `bench/` only (C++ tools, never in the core): the GAP Benchmark Suite (BFS, CC, PR, SSSP, TC), GBBS (BFS, CC, PR, k-core, TC, SSSP), ConnectIt (WCC) and Liu et al.'s k-core (SIGMOD'25), on lj, orkut, twitter-sym and Friendster at 16 threads under the harness, next to ours. These define the M3 exit bar ("at least GAP reference speed, reported against GBBS", {{#m3-00-epic}}; `docs/research/sota-kernels.md` challenge 2)
- [ ] Fairness notes per comparison: same input, same parameters, load time reported separately, what each tool includes; the comparison protocol of {{#m8-03-public-benchmarks}} (same vertex ordering, isolated vertices stated, 64 BFS sources, a 1-thread COST line) applies to these runs too
- [ ] Regression gate: a PR that slows a tracked benchmark beyond its per-case threshold, measured on this host in the coordinator's window (there is no self-hosted runner), needs an explicit waiver. Changed 2026-11-25 from a flat 7% (`docs/research/README.md` M2): two official idle `headline` runs of the same revision already differ by ≥ 7% on 3 of 68 cases (`docs/research/sota-measurement.md` § 3.1), so the gate compares a ratio interval against a δ per case calibrated by an A/A run ({{#m3-18-harness-statistics}}), from interleaved rounds ({{#m3-19-harness-interleave}})
- [ ] **S9, the headline comparison: capacity under a hard memory budget** (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S9). Our distinct claim is a hard budget, and it has no external baseline yet. A curve of time or OOM per kernel against `MemoryMax`:
  - kernels: Friendster PageRank (f64, 20 iterations), WCC, BFS (64 sources), k-core, Leiden. Start small: two kernels and three budgets;
  - `MemoryMax` 2, 4, 8, 16, 32 and 56 GB, cold cache;
  - tools: Everygraph, GBBS on its memory-mapped input (the honest plain-mmap baseline), GAP and NetworKit. A run killed by the limit or the timeout is a result and is reported as such;
  - **data residence proven** for every budgeted row: `pgmajfault`, `workingset_refault_file` and `file` from the cgroup's `memory.stat`, and the run's `read_bytes`. A budgeted run that read fewer bytes than its work counters say it touched, with no major faults, was served from another cgroup's page cache and is invalid;
  - next to each out-of-core time, the time of one sequential read of the same bytes;
  - a **COST line**: the single-thread union-find and PageRank of McSherry et al. (HotOS'15; `frankmcsherry/COST`, Rust, MIT), built under `bench/`, on twitter-2010 and Friendster. Everygraph's COST per kernel is the smallest thread count at which it beats that time.

  Cost: about 50 lines in the harness and one official window (about 3 h, estimate).
- [ ] Published dashboard page generated from result files ({{#m8-03-public-benchmarks}})

**Progress.** PR #250 (part of this issue and of {{#m3-26-benchmark-validity}}) adds `bench/specs/references.toml`: GAP, GBBS, ConnectIt, Parallel-KCore, PASGAL, the COST line, NetworKit, python-igraph, GVE-Leiden (f32 and f64) and the lab Leiden, built from pinned sources under `bench/` (`bench/references/build.sh`), on lj, orkut, roadnet-ca, web-google, twitter-sym and Friendster, at 1 and 16 threads, every output checked against Everygraph's first (`lab/m3-12-reference-runs`). Left: the official run of the spec and the tables from it (ratios only from official rows, with the intervals of {{#m3-18-harness-statistics}}); scipy, DuckDB and the F2 query engines in this spec (they are in `baselines` and `engines`); GBBS SSSP; the degree and reverse-degree orders of the protocol; the regression gate; the dashboard.

**Acceptance**
- Weekly run produces the comparison tables automatically
- Every claim in the README links to a spec and a result file

**Depends on** {{#m0-03-bench-harness}}, {{#m3-18-harness-statistics}}
