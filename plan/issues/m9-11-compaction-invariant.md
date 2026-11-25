---
title: Segments: merging or compacting never changes a result
milestone: M9 · After v0.1
labels: type:feature, area:engine, P1, risk:medium, size:M
---
**Why.** A file built by appends holds a base run and segments ({{#m9-04-temporal-segments}}). The promise for it is already written: results depend on its batches, not on how they are stored — merging or compacting segments never changes a result (`design/DECISIONS.md`, the row on files built by appends, ADR 0028). It is the promise a user of a daily-appended graph would test first, and no dynamic store or incremental system read makes it (`docs/research/sota-dynamic.md` § 5). It must be designed in before any segment code exists, because it is costly to retrofit: a per-node float reduction over neighbours that follows run boundaries gives other bits after a merge. Decided 2026-11-25, before any m9-04 code (`docs/research/README.md` D1).

**Scope**
- [ ] Per-node reductions over neighbours visit the canonical merged order (a k-way merge per list; most lists live in one run) or use order-independent sums (integer or fixed point, {{#m4-02-weights-resolution}})
- [ ] Reduction blocks are node ranges fixed by the node count only, never by file blocks or runs (`docs/spec/eg-adjacency.md` § 4 as amended by ADR 0028)
- [ ] Pair sums over all segments: a pair's sum is the exact sum of its events in every run; statistics recomputed over all runs, not combined from the runs' values (`docs/spec/graph-semantics.md` § 2.7, § 4.3)
- [ ] A merged run is byte-identical to the run an encoder writes for the same pairs, checked by `verify`
- [ ] The test: oracle graphs split into 1–5 runs at random cut points; hashes of PageRank, WCC and Leiden on the split file against the merged one, at 1 and 32 threads (`docs/research/sota-dynamic.md` C3)

**Acceptance**
- The hashes above are equal for every split on every oracle graph, at both thread counts
- The edge-map sources of {{#m3-01-edge-map}} read a multi-run source without a code path per run count

**Pointers.** `docs/research/sota-dynamic.md` § 0.3 A2–A3, § 0.4 S1, S2, S4, § 3 (D14 row), § 4 C3; ADR 0028.

**Depends on** {{#m3-01-edge-map}}
