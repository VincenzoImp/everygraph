---
title: Label propagation as an instant preview
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P2, risk:low, size:S
---
**Why.** On very large graphs, or for a first look, a partition in a couple of scans is useful — for example to produce a quick preview map while Leiden runs, or as a locality ordering.

**Scope**
- [ ] Deterministic synchronous label propagation with tie-breaking by label hash, on the edge-map framework, semi-external capable
- [ ] Clearly labelled as a preview in output provenance; not a substitute for Leiden
- [ ] Evaluate CluStRE Light/Light+ (Chhabra, Peretz, Schulz, SEA 2025; MIT; Q 0.5871 on Friendster in O(n) memory, deterministic for a fixed stream order) as the labelled quick preview and low-budget fallback, against plain synchronous LPA; never substituted silently for Leiden (`docs/research/memory-bounded-algorithms.md`)

**Acceptance**
- Friendster preview partition in < 30 s under a 4 GiB budget
