---
title: Public, reproducible benchmark report
milestone: M8 · CLI, docs and v0.1 launch
labels: type:docs, area:bench, P0, risk:medium, size:M, lab-evidence
---
**Why.** The adoption pattern shared by DuckDB, Polars and Ruff is one reproducible 10–100× claim with open code. Ours must survive hostile review: the first lab's numbers came from a shared machine and single runs.

**Scope**
- [ ] Report generated from `everygraph-bench` result files ({{#m3-12-perf-tracking}}): ingest, kernels, communities (speed *and* quality), out-of-core under cgroup limits, point queries, map build and transfer sizes
- [ ] Hardware: the lab workstation plus one 16 GB laptop and one cloud instance type, so "laptop" claims are measured on a laptop
- [ ] Comparisons with pinned versions and per-tool notes on what is included; invite maintainers of compared tools to review before publication
- [ ] Default-storage numbers come from FINDINGS § M1-13 with their conditions; raw-array numbers are labelled as such
- [ ] One-command reproduction kit with dataset fetchers
- [ ] LDBC Graphalytics as one recognised workload (added 2026-11-25, {{#m1-20-ldbc-graphalytics}}): run by its rules (3 repetitions, timeouts by scale class), PageRank in f64, published with the sentence "These are not LDBC Benchmark Results" and how the runs differ; graph500-30 (447 M nodes, 17 B edges) as a public bigger-than-RAM case for the 61 GB machine
- [ ] A written comparison protocol, applied to every published comparison (decided 2026-11-25, `docs/research/README.md` M4; `docs/research/sota-measurement.md` § 4 challenge 4). Each rule with its source:
  1. the same input file (by hash) and the same vertex ordering for every tool, with results in at least three orderings — input, degree, reverse degree (SoK, Mehrotra et al. 2024: degree order ≈ 40% faster PageRank on twitter-2010, their systems);
  2. n stated with and without isolated vertices, every tool fed the packed graph (Friendster: 65.6 M, not 124 M; SoK: up to 13% on PageRank/CC and > 30% on triangles from isolated vertices);
  3. BFS and SSSP from 64 seeded non-zero-degree sources, the same for every tool (GAP; SoK: BFS from a zero-degree root is 6–10× "faster");
  4. PageRank in f64 with a stated tolerance and iteration count, reported to convergence and per iteration; triangles only on undirected graphs, exact;
  5. load and build (T_l), kernel (T_p) and end to end (T_m) reported separately, per-kernel relabelling charged to the kernel (GAP, Graphalytics);
  6. a 1-thread line and COST for every tool (McSherry et al., HotOS'15), each tool at its best thread count and at a common one, SMT stated;
  7. outputs validated with Graphalytics' match rules, never called "LDBC results";
  8. tools run interleaved in randomized rounds, at least 6 rounds, ratios with intervals ({{#m3-19-harness-interleave}}, {{#m3-18-harness-statistics}}); geometric means only across cases;
  9. peak memory through the cgroup for every tool;
  10. each tool's recommended configuration, checked with its maintainers; hot runs labelled hot, cold runs reported separately;
  11. drive and CPU state recorded ({{#m1-21-drive-state}})
- [ ] The laptop runs (decided 2026-11-25, `docs/research/README.md` M7): a "cool start" and a "back-to-back steady state", with a cooldown to a recorded temperature between rounds and AC or battery recorded. No peer-reviewed laptop study was found (`docs/research/sota-measurement.md` § 2): a boost frequency differs by up to 16% between chips of one model (Acun et al., ICS'16), so a laptop number belongs to one unit, not to the model
- [ ] A cheap check of the published ratios first: BFS on orkut from the hub against 64 seeded non-zero-degree sources, ours and NetworKit, interleaved — does the 4.4–19× range of D1 move?
- [ ] A "where we lose" section (e.g. Gephi on styling small graphs, Neo4j on Cypher, GPUs on raw throughput, BV on web-graph compression)
- [ ] Comparison runners left unavailable by {{#m0-03-bench-harness}}: Neo4j + GDS (needs a server-lifecycle runner: download, import, start bound to 127.0.0.1, stop by PID) and DuckPGQ (needs a DuckDB release compatible with the extension, beside the pinned one); each recorded with tool, version, algorithm and options like the others

**Acceptance**
- An outside person reproduces the headline table on their own machine within 20% using only the kit
- No number in the README or launch post is absent from the report

**Depends on** {{#m0-03-bench-harness}}, {{#m3-12-perf-tracking}}, {{#m4-04-ooc-leiden}}, {{#m5-03-tile-builder}}, {{#m7-01-point-queries}}
