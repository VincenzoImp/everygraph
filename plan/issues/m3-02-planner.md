---
title: Planner: choose regime, threads and decode tier; explain the plan
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P0, risk:medium, size:L, lab-evidence
---
**Why.** "Adapts to any hardware" is the planner. The lab gave it a simple core rule: **choose the regime by the size of the kernel's O(n) state, not by the graph's size** — if the state fits the budget, run on mmap; streams are for a hard bound.

**Rules to implement (each from a measurement)**
- State does not fit the budget → refuse with the minimum budget, or pick a lower-memory variant (e.g. f32 state, streaming union-find instead of Afforest)
- Graph fits comfortably and the workload is random-access heavy → decode to raw CSR (raw PageRank 2× faster on orkut); otherwise stay compressed (equal on twitter, faster scans at high thread counts)
- Budget below the compressed size → semi-external streams (fixed RSS 1.3–1.5 GB on Friendster) or mmap under pressure (same speed, RSS = limit); prefer streams when a hard bound was requested or storage is not fast local NVMe
- Threads per kernel class: physical cores for memory-bound scans (PageRank, components, BFS and k-core gained nothing from 16 → 32 threads); all hardware threads for compute-bound kernels (triangles 2.51 → 1.46 s, Louvain on orkut 0.88 → 0.65 s); fewer for tiny inputs (parallel parse of a small file was slower at 32 threads)
- Views: mask above ~25% selectivity, materialize below ({{#m7-04-views}})

**Scope**
- [ ] Hardware probe: physical cores, RAM available, cgroup limits, storage type and a quick bandwidth probe
- [ ] Per-kernel cost model: O(n) state bytes, scans per iteration, random vs sequential access
- [ ] `explain()` output per {{#m0-12-ux-contract}}; user overrides for every choice
- [ ] Plan recorded in result provenance

**Acceptance**
- On the lab machine the planner's choice is within 10% of the best manually chosen configuration for PageRank, WCC and BFS on orkut, twitter-sym and Friendster at budgets of 2, 8, 16 GiB and unlimited
- It never selects a plan that is then OOM-killed ({{#m3-10-no-oom-ci}})

**Depends on** {{#m2-04-arrays-accountant}}, {{#m3-01-edge-map}}
