---
title: Planner: choose regime, threads and decode tier; explain the plan
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P0, risk:medium, size:L, lab-evidence
---
**Why.** "Adapts to any hardware" is the planner. The lab gave it a simple core rule: **choose the regime by the size of the kernel's O(n) state, not by the graph's size** — if the state fits the budget, run on mmap; streams are for a hard bound.

**Rules to implement (each from a measurement)**
- State does not fit the budget → refuse with the minimum budget. The planner may switch by itself only between strategies that give the identical result (e.g. streaming union-find instead of Afforest); a variant that changes the result (f32 state) is never substituted: it is offered in the refusal as a way forward, and only where it exists (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises")
- Decode tier (ADR 0004): compressed for every kernel; build the raw cache only if R + S ≤ M with headroom and the session repeats traversals or point lookups past the break-even (arithmetic on #118's measurements: about 6–7 BFS runs at 16 threads, 280–970 shortest-path queries, 0.85–8.7 M neighbour lookups, orkut to Friendster); never for PageRank, WCC, k-core, triangles, Leiden; if raw does not fit and compressed does, stay compressed (raw through mmap 1.2–5.4× slower on scans)
- Budget below the compressed size → semi-external streams (fixed RSS 1.3–1.5 GB on Friendster) or mmap under pressure (same speed, RSS = limit); prefer streams when a hard bound was requested or storage is not fast local NVMe
- Threads per kernel class: physical cores for memory-bound scans (PageRank, components, BFS and k-core gained nothing from 16 → 32 threads); all hardware threads for compute-bound kernels (triangles 2.51 → 1.46 s, Louvain on orkut 0.88 → 0.65 s); fewer for tiny inputs (parallel parse of a small file was slower at 32 threads)
- Views: mask or materialized by a rule that {{#m7-04-views}} derives from measurement (number of passes, memory budget and selectivity; the earlier "~25%" threshold was not a measurement — ADR 0010)

**Scope**
- [ ] Hardware probe: physical cores, RAM available, cgroup limits, storage type and a quick bandwidth probe
- [ ] Per-kernel cost model: O(n) state bytes, scans per iteration, random vs sequential access
- [ ] `explain()` output per {{#m0-12-ux-contract}}; user overrides for every choice
- [ ] Plan recorded in result provenance
- [ ] Plans and refusals as `docs/ux-contract.md` § 3: documents valid against its closed plan, error and event schemas; only ways forward that exist (P-15); a budget above the machine's memory is a usage error (P-16), one above free memory is honoured with a line (P-17), memory-backed scratch counts against the budget (P-18); estimate and actual in each result's provenance (P-9)

**Acceptance**
- On the lab machine the planner's choice is within 10% of the best manually chosen configuration for PageRank, WCC and BFS on orkut, twitter-sym and Friendster at budgets of 2, 8, 16 GiB and unlimited
- On orkut and twitter-sym the planner switches to the decode tier after the break-even count of BFS runs, not before
- It never selects a plan that is then OOM-killed ({{#m3-10-no-oom-ci}})
- For every kernel, the result hash is the same in each regime the planner can choose (in memory, edges on disk, state on disk), tested by forcing each regime on a tier-1 graph (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises")

**Depends on** {{#m2-04-arrays-accountant}}, {{#m3-01-edge-map}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Approved defaults (`design/preview/choices.md` A5, `cli.md` Choice 4): all physical cores and half of free memory unless told otherwise; the plan is always shown before a long operation, in the form typeset in `design/preview/style/``terminal.html` ("From a file to a map") and `notebook.html` (the plan card).
