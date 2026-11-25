---
title: Out-of-core Leiden: sequential aggregation and spilled level-1 graph
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P0, risk:high, size:L, lab-evidence
---
**Question.** Can Leiden run on a graph larger than RAM with bounded memory and acceptable slowdown?

**What the lab found.** The O(n) state is small (21–53 B/node). Two things break the semi-external story: (1) aggregation visits vertices grouped by community — one random adjacency read per vertex, twice in low-memory mode — and takes 58–69% of the time on Friendster; (2) Leiden's level-1 quotient graph keeps 22–44% of the input entries (≈ 12 GB on Friendster). In the scaled-down test (orkut with the limit near the edge-array size) the run was 5.6× slower; below that, 51× slower or OOM-killed.

**Where it fails at full scale (tracer, {{#m1-01-e2e-tracer}}, 2026-11-25, idle; `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 9–15; logs in `lab/m1-01-tracer/results/`).** On Friendster under `MemoryMax=16G` (32 threads, async, 1 iteration, f32 weights, `--lowmem`, CSR memory-mapped, fan-out cap 64) both Leiden and the capped Louvain are killed in **pass-0 aggregation, after the local moves and refinement complete**: Leiden took 65.6 M nodes to 20.9 M communities (moves 37.8 s, refinement 6.3 s) and was killed at 460.5 s; Louvain reached 10.5 M communities and was killed at 648.7 and 648.2 s. Both were thrashing on the mapped graph: **1.80 TB read (Leiden) and 2.72–2.74 TB (Louvain)** for a 15 GB graph. Leiden's level-1 quotient graph alone is about 22 GB (arithmetic: 2.77 B entries × 8 B). With hierarchy output the capped Leiden is killed even at 56 GiB (55.8 GiB anonymous, after pass 4, `logs/communities-leiden-56G.warmup1.measure.json`) because the prototype keeps every level's quotient graph in memory (`keep_graphs` with `--out-hier`: about 51.5 GiB of quotient graphs by arithmetic; not tested without `--out-hier`). Plain Louvain fits (4.2 GiB anonymous, Q 0.6038; aggregation 74.6 s of 104.3 s, `b2_louvain_plain_16G.json`) but is no substitute: its hierarchy has a parent with 18.7 M children that the layout cannot use ({{#m1-05-hierarchy-shaping}}). This is the first blocker of the 16 GiB claim.

**Method**
- [ ] Replace member-order aggregation with a sequential one: one streaming pass over the adjacency emitting (community(u), community(v), w) triples into range-partitioned buckets, then per-bucket sort/combine — the same bucket machinery as ingest ({{#m2-08-csr-build}})
- [ ] Keep the level-1 graph on disk in the standard compressed format and run level 1 through the same streaming source; from level 2 on it normally fits in RAM
- [ ] Budget every allocation through a memory-accountant stub
- [ ] Budget or spill the level-1 quotient graph produced by pass-0 aggregation — the point where the tracer's 16 GiB runs died, with 10.5–20.9 M level-1 communities (`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 9, 11–12)
- [ ] Also run with hierarchy output on and the map's fan-out cap ({{#m1-05-hierarchy-shaping}}), not only the plain partition: each level's quotient graph streamed to disk ({{#m4-06-hierarchy-artifact}}), never all kept in memory (the tracer's capped Leiden was killed at 56 GiB that way, `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 10 and its warm-up log)
- [ ] Record bytes read per phase: the tracer's 16 GiB failures read 1.80–2.74 TB before the kill (`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 9, 11–12)
- [ ] Measure on Friendster at 34 GiB, 16 GiB and 8 GiB limits: time per phase, anonymous peak, modularity, disconnected communities; and on orkut with a limit of 50% of the edge array
- [ ] Compare against the in-RAM run (154.5 s, Q 0.6033, 31,960 communities; 32 threads, shared machine. Idle, one async iteration at 16 threads: 82.4 s raw / 82.0 s compressed, Q 0.6050 / 0.6008, one repetition each, #118 — not comparable with 154.5 s as a speed-up). Re-run the in-RAM reference idle, in the same configuration as the out-of-core runs

**Success.** Friendster under 16 GiB: completes, Q ≥ 0.60, 0 disconnected communities, ≤ 2.5× the in-RAM time. If not reachable, state the smallest budget at which it is and document the limit in the product.

**Feeds.** {{#m4-04-ooc-leiden}}, ADR for D11.

**Pointers.** `lab/b2-leiden/src/main.rs` (`--lowmem`), FINDINGS § B2 "Memory / out-of-core".
