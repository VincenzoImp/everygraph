---
title: M3 epic: engine and kernels
milestone: M3 · Engine and kernels
labels: type:epic, area:engine, P0
---
Tracking issue for milestone M3.

**Goal.** A planner, a memory accountant and one edge-map framework that run every kernel in three regimes (mmap warm, mmap under pressure, semi-external streams) with identical results, at any thread count, inside the memory budget.

**Lab baseline (idle machine, orkut).** PageRank 20 iterations 3.57 s at 1 thread → 0.37 s at 16; WCC 81 → 13 ms; BFS 99 → 22 ms; k-core 1.35 → 0.28 s; triangles 1.46 s at 32 threads. NetworKit 11.3 on the same idle machine: PageRank 2.79 s, parallel components 247 ms, k-core 1.23 s, triangles 24.0 s. Friendster PageRank: 3.4 s/iteration in RAM; with the raw-format streaming reader 3.7 and 3.6 s/iteration under 8 GB and 2 GB limits (32 threads).

**Default storage (compressed, #118, idle, graph in heap; FINDINGS § M1-13).** orkut PageRank f32 20 iterations 3.65 s at 1 thread → 0.266 s at 16 (raw in the same harness 3.66 → 0.366); WCC 77 → 17.2 ms; BFS 196 → 28.0 ms; k-core 1.28 s at 1 thread (16 threads not trusted: 0.22–0.36 s in every configuration); triangles 1.44 s at 32 threads. Friendster PageRank f32 at 16 threads: 3.19 s/iteration in RAM (raw 3.36); 4.31 s/iteration under a 2 GB limit through plain mmap. The 2 GB exit criterion has no margin without {{#m3-03-semi-external}} (1.35× measured through plain mmap).

**Exit criteria**
- [ ] Framework and planner ({{#m3-01-edge-map}}, {{#m3-02-planner}}, {{#m3-03-semi-external}})
- [ ] Kernels: {{#m3-04-pagerank}}, {{#m3-05-components}}, {{#m3-06-traversal}}, {{#m3-07-kcore-degrees}}, {{#m3-08-triangles}}; eigenvector centrality in {{#m3-04-pagerank}} (the sampled centralities of {{#m3-09-centrality}} come after 0.1, M9)
- [ ] Guarantees enforced in CI: {{#m3-10-no-oom-ci}}, {{#m3-11-determinism-ci}}
- [ ] Performance tracked against references ({{#m3-12-perf-tracking}}), with intervals and interleaved rounds ({{#m3-18-harness-statistics}}, {{#m3-19-harness-interleave}}); the speed gaps of closed issues collected and closed in {{#m3-22-performance-pass}}, whose official numbers come from the milestone window (CLAUDE.md § 6)
- [ ] Testing from the second literature review (2026-11-25, `docs/research/README.md` D): {{#m3-16-metamorphic-oracle}}, {{#m3-17-cargo-mutants}}
- [ ] {{#m3-33-harness-iteration-level}}, {{#m3-35-harness-zero-reference}}
- [ ] CI: {{#m3-34-test-compile-weight}}, {{#m3-36-harness-oom-retries}}
- [ ] From the state-of-the-art review (2026-11-26): {{#m3-25-memory-headroom}}, {{#m3-26-benchmark-validity}}, {{#m3-27-frontier-counters}}
- [ ] Milestone close (CLAUDE.md § 6, § 10): the dogfood pass, the cleanup and test audit, the official measurement window, and the report `docs/reports/M3.md`: {{#m3-23-milestone-close}}, with {{#m3-22-performance-pass}} for the window
- [ ] Friendster PageRank under a 2 GB limit within 1.35× of in-RAM; every kernel the GAP Benchmark Suite implements (BFS, SSSP, PageRank, components, triangles) at least as fast as the GAP reference on this machine, at equal threads, on orkut and Friendster; every kernel reported against GBBS (WCC also against ConnectIt, k-core against Liu et al.); the reference runs are {{#m3-12-perf-tracking}}'s. Changed 2026-11-25 from "≥ NetworKit speed": NetworKit is not the best known for these kernels, and every cross-paper ratio in `docs/research/sota-kernels.md` is from 2–4-socket machines (challenge 2)
