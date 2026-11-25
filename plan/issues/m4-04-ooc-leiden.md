---
title: Leiden beyond RAM
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P0, risk:high, size:L, lab-evidence
---
**Why.** This is the largest open technical risk (DESIGN § 5.1). Today Leiden on Friendster needs 22.6 GiB of anonymous memory; the only run that fit 16 GiB was Louvain with a poor partition. Friendster Leiden, one iteration, 16 threads, idle: 82 s; anonymous peak 37.0 GiB raw / 34.3 compressed with the graph in heap — compression alone does not bring it under 16 GiB (#118).

**Scope.** Productionize the outcome of {{#m1-02-ooc-aggregation}}:
- [ ] Level 0 over the streaming source with O(n) state reserved through the accountant
- [ ] Sequential bucket aggregation; level-1 graph written to disk and processed through the same streaming source
- [ ] Planner integration: per-level regime choice, preflight estimate of state + level-1 size (22–44% of the input entries in the lab)
- [ ] Visiting order: block-permuted order reads the edge array in runs of 64 consecutive nodes; raise the block to ~1k nodes for streamed levels (no quality loss measured up to 1,024) and schedule blocks to follow disk order within a pass
- [ ] Included in the no-OOM matrix ({{#m3-10-no-oom-ci}})
- [ ] The visiting order out of core is a fixed function of the graph and the seed, never of the budget (otherwise "same result for any budget" breaks). Candidates from the survey (`docs/research/memory-bounded-algorithms.md`), to be tested in {{#m1-02-ooc-aggregation}}: decorrelation by a seeded relabelling at ingest with storage-order visits; a windowed block permutation of fixed width; the active set as a bitmap swept in disk order with block skipping; level-1 never materialized (stream level 0 through community[], Graphyti); bumped two-phase hashing in aggregation (TeraPart). If one changes ADR 0015's order, the ADR is amended

**Acceptance**
- Friendster under `MemoryMax=16G`: completes, Q ≥ 0.60, 0 disconnected, ≤ 2.5× the in-RAM time
- orkut with a limit of 50% of its edge array: ≤ 3× the in-RAM time (lab: 5.6× at 100%, OOM below)
- Deterministic mode works out of core with the same hash as in RAM

**Depends on** {{#m1-02-ooc-aggregation}}, {{#m3-03-semi-external}}, {{#m4-03-parallel-aggregation}}
