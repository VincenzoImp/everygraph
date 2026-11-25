---
title: Layout engine: hierarchy-constrained, deterministic, parallel
milestone: M5 · The map
labels: type:feature, area:map, P0, risk:medium, size:L, lab-evidence
---
**Why.** The lab layout (disc per community with area ∝ node count, small force solve among siblings inside the parent, recursive) is cheap — `O(m·D + Σk²)` with bounded fan-out — and readable, while an embedding + PCA layout was a blob at every zoom (edge-length ratio 0.244 vs 0.014).

**Scope**
- [ ] Production implementation of the layout chosen in {{#m1-07-layout-area}} (disc packing or power-diagram cells)
- [ ] Input: map hierarchy ({{#m4-07-map-hierarchy}}) and its quotient graphs; output: coordinates and radii/cells per tree node, node coordinates at the leaves — stored as a derived artifact
- [ ] Parallelize the serial apply pass (lab scaling 6–7× at 32 threads; subtrees are independent)
- [ ] Deterministic by construction: fixed iteration order, no thread-dependent float merges (the lab found and fixed one)
- [ ] Leaf placement: a few force iterations for small communities, deterministic neighbor-guided packing for large ones
- [ ] Memory bounded by the accountant; out-of-core for leaves (process subtree by subtree)
- [ ] Stable under small graph changes where possible (seeded from parent positions)

**Acceptance (lab machine)**
- lj ≤ 3 s and orkut ≤ 8 s at 16 threads; ≥ 10× scaling from 1 to 16 threads
- Byte-identical coordinates at 1, 4, 16 threads
- Friendster layout completes under a 16 GiB budget (the lab extrapolated ~20 GB)

**Pointers.** `lab/e-map`, FINDINGS § E.

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Bubbles (nested discs) are the approved default map style; "cells" stays a possible second view and is not required for 0.1 (`design/preview/style/``viewer.html`, `data.html` §4–5).
