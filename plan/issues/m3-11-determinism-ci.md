---
title: Determinism in CI: hashes across thread counts and platforms
milestone: M3 · Engine and kernels
labels: type:infra, area:engine, P0, risk:medium, size:S, lab-evidence
---
**Why.** Deterministic output is a headline guarantee (DESIGN D14). The lab already showed identical results across native, wasm32, wasm64, Node and Chromium and across 1–32 threads for the simple kernels, identical Leiden/Louvain hashes at 1–32 threads, and byte-identical layout and tile files — after one thread-dependent float merge was found and fixed in the layout. Such regressions are easy to introduce.

**Scope**
- [ ] Canonical result hashing (FNV/BLAKE3 over output columns) exposed by every kernel
- [ ] CI job: each kernel on tier-1 graphs at 1, 2, 3, 8 threads and oversubscribed (32 workers on 4 cores), plus native vs wasm32; all hashes must match
- [ ] The same hashes under each execution regime (forced: in memory, edges on disk, state on disk) and under two memory budgets, and for a view run as a mask and materialized (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises"). Results cut short by `timeout` are exempt
- [ ] Floating point: fixed chunking and reduction order; make results bit-identical across the supported CPU architectures (no fused multiply-add or platform maths routines in result-bearing arithmetic; x86 vs ARM); an exception needs an ADR that narrows the promise of `docs/api-sketch.md` § 2.11
- [ ] The `fast` (non-deterministic) modes are labelled as such in output provenance
- [ ] Large-graph references: the `ldbc` profile proposed by {{#m1-20-ldbc-graphalytics}} (LDBC Graphalytics reference outputs for BFS, WCC, PageRank in f64, CDLP, LCC, SSSP), run in the nightly job, in RAM and under a budget below the graph's size (added 2026-11-25, `docs/research/README.md` B12)
- [ ] A scheduled nightly workflow that runs `oracle/run-nightly.sh` (fetches tier 1, checks all golden files, nightly corpus, seeded bugs) and opens an issue when it fails; billed minutes stated (the script took 18 min on the lab machine)

**Acceptance**
- A PR that makes a reduction order depend on the thread count fails CI

**Depends on** {{#m3-01-edge-map}}, {{#m0-02-ci}}
