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
- [ ] Deterministic by construction: fixed iteration order, no thread-dependent float merges (the lab found and fixed one). Angles and jitter through `everygraph_core::math` and `everygraph_core::rng` ({{#m2-26-core-math}}, {{#m2-27-counter-rng}}), not `f64::cos`/`sin` (`lab/e-map/gmap/src/layout.rs` l. 139–140) or SplitMix64 streams, so that native and wasm32 give the same bytes
- [ ] Leaf placement: a few force iterations for small communities, deterministic neighbor-guided packing for large ones
- [ ] Memory bounded by the accountant; out-of-core for leaves (process subtree by subtree)
- [ ] Never load the graph into the heap. On Friendster under 16 GiB the lab layout was killed with the graph in the heap (15.7 s) and fit with it memory-mapped (3.7 GiB anonymous), but took 405.1 s against 69.6 s with 56 GiB (5.8×) and re-read 733 GB, with the kernel's default readahead (no `madvise`; `MADV_RANDOM` not tried) ({{#m1-01-e2e-tracer}}, `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 18–19, 22–23). Read the graph in locality order, and lay out the upper levels from the hierarchy's quotient graphs ({{#m4-06-hierarchy-artifact}}) rather than from the full adjacency
- [ ] Bounded memory for a parent with millions of children: the lab's derived-region step builds an unbounded map over sibling pairs when a parent has more than 384 children, and on the plain Louvain hierarchy (a parent with 18.7 M children) it was killed during depth 3 even with 56 GiB (`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 17, 20–21)
- [ ] Stable under small graph changes where possible (seeded from parent positions)
- [ ] Read in hierarchy order (added 2026-11-25, `docs/research/sota-maps.md` challenge 4). With node ids in hierarchy (DFS) order ({{#m1-09-hierarchy-order}}, {{#m1-14-relabel-at-ingest}}), every parent's members are a contiguous id range and every depth is one sequential pass. Otherwise, sort the members per parent first. Inside finest communities above a size threshold, nodes are placed along a space-filling curve (Hilbert or Gosper) of their sub-order (Muelder–Ma 2008: Θ(n), deterministic) instead of a force solve, which gives near-uniform clouds today (`lab/e-map/REPORT.md` Q4). A curve shows order, not internal topology, so the threshold is measured on lj (edge ratio inside giant communities, curve against force)
- [ ] Random access that remains goes through the block cache ({{#m2-23-block-cache}}) if {{#m1-15-mmap-hints-block-cache}} adopts it, not through plain mmap under pressure

**Acceptance (lab machine)**
- The metric suite of {{#m1-07-layout-area}} (distributions, NP on quotient levels, stress at optimal scale, occupied area) reported on lj and orkut, against the published-layout baseline of {{#m1-18-layout-baseline}}
- lj ≤ 3 s and orkut ≤ 8 s at 16 threads; ≥ 10× scaling from 1 to 16 threads
- Byte-identical coordinates at 1, 4, 16 threads
- Friendster layout completes under a 16 GiB budget (the lab extrapolated ~20 GB; measured 2026-11-25: 69.6 s with 17.7 GiB anonymous with 56 GiB, graph in the heap; under 16 GiB only memory-mapped, 405.1 s and 733 GB read; `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 19, 22–23), with time and bytes read reported against those

**Pointers.** `lab/e-map`, FINDINGS § E.

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Bubbles (nested discs) are the approved default map style; "cells" stays a possible second view and is not required for 0.1 (`design/preview/style/viewer.html`, `data.html` §4–5).
