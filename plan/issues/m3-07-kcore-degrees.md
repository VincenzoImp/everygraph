---
title: k-core decomposition and degree statistics
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P1, risk:low, size:S, lab-evidence
---
**Scope**
- [ ] Degree columns `in_degree`, `out_degree`, `degree` (distinct neighbors; `degree` counts each neighbor once whatever the direction) and `weighted`; on a view they are computed by a planned pass. Also, exact histogram, quantiles, isolates — computed at ingest and stored
- [ ] k-core with PKC-style peeling and task-local stacks (the lab's first level-synchronous version scaled *negatively*: 1.39 s at 8 threads, 1.93 s at 32; PKC: 0.28 s at 8)
- [ ] Core-number column, shell sizes, k-core subgraph as a view
- [ ] Note: k-core does not scale past 8 threads in the prototype (0.28 → 0.30 → 0.37 s at 8/16/32); investigate or cap threads
- [ ] A bounded-memory path for core numbers: SemiCore* (Wen et al., ICDE 2016; exact, unique fixpoint, O(n) memory, re-reads only nodes whose estimate may change) over the edge-map's semi-external source; identical result to the in-RAM peel (`docs/research/memory-bounded-algorithms.md`). Without it the planner refuses k-core when the graph does not fit, which leaves promise 2 uncovered

**Acceptance**
- Core numbers equal NetworkX on oracle graphs; max core 360 (lj), 253 (orkut), 304 (Friendster)
- orkut ≤ 0.3 s (needs a stable measurement: #118 saw 0.22–0.36 s at 16 threads in every configuration); Friendster ≤ 10 s

**Depends on** {{#m3-01-edge-map}}
