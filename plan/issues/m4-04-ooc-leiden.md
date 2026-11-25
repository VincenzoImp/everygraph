---
title: Leiden beyond RAM
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P0, risk:high, size:L, lab-evidence
---
**Why.** This is the largest open technical risk (DESIGN § 5.1). Today Leiden on Friendster needs 22.6 GiB of anonymous memory; the only run that fit 16 GiB was Louvain with a poor partition (Q 0.4647; not reproduced by the tracer, which got Q 0.6038 but a hierarchy the map cannot use, {{#m1-03-louvain-collapse}}). Friendster Leiden, one iteration, 16 threads, idle: 82 s; anonymous peak 37.0 GiB raw / 34.3 compressed with the graph in heap — compression alone does not bring it under 16 GiB (#118). The tracer ({{#m1-01-e2e-tracer}}, 2026-11-25, idle, `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 9–15) located the failure: under 16 GiB both Leiden and the fan-out-capped Louvain are killed in pass-0 aggregation after the local moves and refinement complete, thrashing on the mapped graph (1.80 TB and 2.72–2.74 TB read); with hierarchy output the capped Leiden is killed even at 56 GiB (55.8 GiB anonymous, `lab/m1-01-tracer/results/logs/communities-leiden-56G.warmup1.measure.json`) because every level's quotient graph is kept in memory (about 51.5 GiB by arithmetic; not tested without hierarchy output).

**Scope.** Productionize the outcome of {{#m1-02-ooc-aggregation}}:
- [ ] Level 0 over the streaming source with O(n) state reserved through the accountant
- [ ] Sequential bucket aggregation; level-1 graph written to disk and processed through the same streaming source
- [ ] Planner integration: per-level regime choice, preflight estimate of state + level-1 size (22–44% of the input entries in the lab)
- [ ] Visiting order: block-permuted order reads the edge array in runs of 64 consecutive nodes; raise the block to ~1k nodes for streamed levels (no quality loss measured up to 1,024) and schedule blocks to follow disk order within a pass
- [ ] Each level's quotient graph written to disk as it is produced and dropped from memory ({{#m4-06-hierarchy-artifact}}); memory does not grow with the number of levels (the tracer's capped Leiden with every level kept: killed at 56 GiB, `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 10 and its warm-up log)
- [ ] Included in the no-OOM matrix ({{#m3-10-no-oom-ci}})
- [ ] The visiting order out of core is a fixed function of the graph and the seed, never of the budget (otherwise "same result for any budget" breaks). Candidates from the survey (`docs/research/memory-bounded-algorithms.md`), to be tested in {{#m1-02-ooc-aggregation}}: decorrelation by a seeded relabelling at ingest with storage-order visits; a windowed block permutation of fixed width; the active set as a bitmap swept in disk order with block skipping; level-1 never materialized (stream level 0 through community[], Graphyti); bumped two-phase hashing in aggregation (TeraPart). If one changes ADR 0015's order, the ADR is amended

**Acceptance**
- Friendster under `MemoryMax=16G`: completes, Q ≥ 0.60, 0 disconnected, ≤ 2.5× the in-RAM time
- The same with the map hierarchy output on (fan-out-bounded levels for {{#m4-07-map-hierarchy}}): completes under `MemoryMax=16G`, with bytes read reported (the tracer's map-shaped runs were killed in pass-0 aggregation after 1.80–2.74 TB of reads, `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 9, 11–12)
- orkut with a limit of 50% of its edge array: ≤ 3× the in-RAM time (lab: 5.6× at 100%, OOM below)
- Deterministic mode works out of core with the same hash as in RAM

**Depends on** {{#m1-02-ooc-aggregation}}, {{#m3-03-semi-external}}, {{#m4-03-parallel-aggregation}}
