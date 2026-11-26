---
title: Layout: recover wasted area and make distances mean something
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:map, P1, risk:medium, size:M, lab-evidence
---
**Question.** Nested discs leave nodes covering only 1–3% of the map and inter-continent distances carry little meaning. Is there a layout with the same cost and determinism that uses the plane better?

**Candidates**
- [ ] Power-diagram / Voronoi treemap cells with area ∝ node count (weighted centroidal Voronoi inside each parent). Sharpened 2026-11-25 (`docs/research/sota-maps.md` challenge 2): power diagram per parent (Lloyd iterations plus weight adaptation, Nocaj–Brandes), seeded by the stress positions below; cells clipped to a convex parent stay convex; the parent's inscribed disc as fallback when a cell degenerates; floating point in a fixed order for byte identity. The viewer keeps the approved bubble look by drawing each child's **largest inscribed circle** (NCP does power diagram → circles), or the "cells" style already in the preview. Expected by arithmetic, to be measured: today children get φ = 0.72 of the parent's area per level, so a node at depth 7–9 gets about 3.6–6.9% of the map before packing loss; space-filling cells keep 100% at every inner level
- [ ] Disc packing with compaction passes and sibling ordering by quotient affinity
- [ ] Stress-based placement of siblings on the parent's quotient graph, kept during packing (sharpened 2026-11-25, `docs/research/sota-maps.md` challenge 1). Target distance d_ij from inter-community affinity, e.g. −log of the normalized cut weight, floored at r_i + r_j + gap. Exact classical MDS start (sign fixed by a rule), then a stress descent in the style of Zheng et al. with a seeded counter-based shuffle and weights scaled by child size (Ortmann's s/d²), then the existing overlap sweep. Children start from the parent's coordinates of their quotient neighbours. Deterministic if the shuffle is counter-based and each parent is solved sequentially, with parents in parallel as now. A/B on lj and orkut against the current force solve, same hierarchy, 16 threads
- [ ] Close the global gap left by {{#m1-18-layout-baseline}} (ADR 0017 amendment). With stress per parent, Spearman (layout vs hop distance) is 0.33 on orkut against DRGraph's 0.55, and edges between top regions (21–24% of edges) are 0.88 (orkut) and 1.04 (lj) of a random pair against 0.16 and 0.23 (`lab/m1-18-layout-baseline/results/README.md`, `crosslevel_<g>.json`). Hypothesis, not measured: the gap comes from where members with outside links sit inside each region. On orkut DRGraph's top regions are as compact as ours, and a rigid rotation per sibling group did not help. Measure a placement of members towards the region's outside neighbours inside each region, against `hcls`, on lj and orkut
- [ ] Redesign or drop the "pull towards external neighbors" term (no measurable gain in the lab: ratio 0.226 with, 0.223 without)
- [ ] Parallelize the serial apply pass (layout scales only 6–7× at 32 threads)

**Metrics** (widened 2026-11-25, `docs/research/sota-maps.md` § 3 D17 and challenge 6). Report distributions, not only medians:
- occupied area fraction (node discs and finest regions), zmax, tile count and file size;
- median, mean and p90 edge length ÷ random-pair distance. Today on lj the p90 is 0.430 against a random-pair median of 0.427 (`lab/e-map/results/eval_lj_hcl.json`), so the longest 10% of edges are as long as random pairs;
- NP₁/NP₂ neighbourhood preservation on 10⁵ sampled nodes, used on the quotient levels: NP is ≤ 0.04 on social graphs for every published method (t-FDP, SNAP-tFDP papers), so it does not separate methods at node level;
- stress at optimal scale on sampled pairs (raw normalized stress is scale-dependent; Smelser et al.);
- the coefficient of variation of edge length (Hachul–Jünger);
- per level, sibling adjacency against quotient weight;
- layout time and memory on lj and orkut; byte-identical output across thread counts.

**Done when.** A recommended layout with images and numbers on lj and orkut, and a go/no-go on replacing disc packing in {{#m5-01-layout}}. D17 is re-decided only together with the published-layout baseline of {{#m1-18-layout-baseline}}: today's D17 evidence compares with an 8-dimensional FastRP + PCA layout, not a published large-graph layout.

**Pointers.** `lab/e-map`, FINDINGS § E; literature: GMap, Voronoi treemaps, FM3/sfdp multilevel.
