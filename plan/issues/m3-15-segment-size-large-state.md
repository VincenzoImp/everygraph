---
title: State arrays above 1 GiB: segment size on 64-bit native targets
milestone: M3 · Engine and kernels
labels: type:decision, area:engine, P2, risk:low, size:S
---
**Why.** `SegVec` ({{#m2-04-arrays-accountant}}, PR #154) caps segments at 1 GiB, as ADR 0002 requires for the browser's 2 GiB allocation limit. Random access spread over many segments measured 1.7–1.8× slower (2^16-element segments, exploratory); it matters only for state arrays above 1 GiB (f32 state beyond 2^28 nodes). ADR 0002 already names unbounded segments on 64-bit native targets as an option.

**Scope**
- [ ] Measure random-access kernels with 1 GiB segments vs one allocation on a graph with more than 2^28 nodes (synthetic), interleaved A/B
- [ ] Decide by ADR: keep the cap everywhere, or lift it on 64-bit native targets (keeping results identical)
