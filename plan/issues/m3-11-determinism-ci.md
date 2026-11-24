---
title: Determinism in CI: hashes across thread counts and platforms
milestone: M3 · Engine and kernels
labels: type:infra, area:engine, P0, risk:medium, size:S, lab-evidence
---
**Why.** Deterministic output is a headline guarantee (DESIGN D14). The lab already showed identical results across native, wasm32, wasm64, Node and Chromium and across 1–32 threads for the simple kernels, identical Leiden/Louvain hashes at 1–32 threads, and byte-identical layout and tile files — after one thread-dependent float merge was found and fixed in the layout. Such regressions are easy to introduce.

**Scope**
- [ ] Canonical result hashing (FNV/BLAKE3 over output columns) exposed by every kernel
- [ ] CI job: each kernel on tier-1 graphs at 1, 2, 3, 8 threads and oversubscribed (32 workers on 4 cores), plus native vs wasm32; all hashes must match
- [ ] Floating point: fixed chunking and reduction order; document where results are bit-identical vs tolerance-bounded across CPU architectures (x86 vs ARM FMA differences)
- [ ] The `fast` (non-deterministic) modes are labelled as such in output provenance

**Acceptance**
- A PR that makes a reduction order depend on the thread count fails CI

**Depends on** {{#m3-01-edge-map}}, {{#m0-02-ci}}
