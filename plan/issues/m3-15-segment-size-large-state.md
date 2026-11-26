---
title: State arrays above 1 GiB: segment size on 64-bit native targets
milestone: M3 · Engine and kernels
labels: type:decision, area:engine, P2, risk:low, size:S
---
**Why.** `SegVec` ({{#m2-04-arrays-accountant}}, PR #154) uses 1 GiB segments by default, inside ADR 0002's 2 GiB limit for the browser. Pull PageRank with its state in 2^16-element segments, a stand-in for state arrays above one segment, took 1.71–1.84× the time of `Vec` on lj and orkut (idle, 16 threads; `bench/results/segvec/20261125T210132Z-14b90a211e8d.jsonl`, ADR 0002). It matters only for state arrays above 1 GiB (f32 state beyond 2^28 nodes). ADR 0002's "what would change our mind" covers this case: more than 5% overhead from segmentation that cannot be removed for native builds.

**Scope**
- [ ] Measure random-access kernels with 1 GiB segments vs one allocation on a graph with more than 2^28 nodes (synthetic), interleaved A/B
- [ ] Decide by ADR: keep the cap everywhere, or lift it on 64-bit native targets (keeping results identical)
