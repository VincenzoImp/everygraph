---
title: Out-of-core Leiden: sequential aggregation and spilled level-1 graph
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P0, risk:high, size:L, lab-evidence
---
**Question.** Can Leiden run on a graph larger than RAM with bounded memory and acceptable slowdown?

**What the lab found.** The O(n) state is small (21–53 B/node). Two things break the semi-external story: (1) aggregation visits vertices grouped by community — one random adjacency read per vertex, twice in low-memory mode — and takes 58–69% of the time on Friendster; (2) Leiden's level-1 quotient graph keeps 22–44% of the input entries (≈ 12 GB on Friendster). In the scaled-down test (orkut with the limit near the edge-array size) the run was 5.6× slower; below that, 51× slower or OOM-killed.

**Method**
- [ ] Replace member-order aggregation with a sequential one: one streaming pass over the adjacency emitting (community(u), community(v), w) triples into range-partitioned buckets, then per-bucket sort/combine — the same bucket machinery as ingest ({{#m2-08-csr-build}})
- [ ] Keep the level-1 graph on disk in the standard compressed format and run level 1 through the same streaming source; from level 2 on it normally fits in RAM
- [ ] Budget every allocation through a memory-accountant stub
- [ ] Measure on Friendster at 34 GiB, 16 GiB and 8 GiB limits: time per phase, anonymous peak, modularity, disconnected communities; and on orkut with a limit of 50% of the edge array
- [ ] Compare against the in-RAM run (154.5 s, Q 0.6033, 31,960 communities; 32 threads, shared machine. Idle, one async iteration at 16 threads: 82.4 s raw / 82.0 s compressed, Q 0.6050 / 0.6008, one repetition each, #118 — not comparable with 154.5 s as a speed-up). Re-run the in-RAM reference idle, in the same configuration as the out-of-core runs

**Success.** Friendster under 16 GiB: completes, Q ≥ 0.60, 0 disconnected communities, ≤ 2.5× the in-RAM time. If not reachable, state the smallest budget at which it is and document the limit in the product.

**Feeds.** {{#m4-04-ooc-leiden}}, ADR for D11.

**Pointers.** `lab/b2-leiden/src/main.rs` (`--lowmem`), FINDINGS § B2 "Memory / out-of-core".
