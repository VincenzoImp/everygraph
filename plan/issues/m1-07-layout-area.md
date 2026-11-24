---
title: Layout: recover wasted area and make distances mean something
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:map, P1, risk:medium, size:M, lab-evidence
---
**Question.** Nested discs leave nodes covering only 1–3% of the map and inter-continent distances carry little meaning. Is there a layout with the same cost and determinism that uses the plane better?

**Candidates**
- [ ] Power-diagram / Voronoi treemap cells with area ∝ node count (weighted centroidal Voronoi inside each parent)
- [ ] Disc packing with compaction passes and sibling ordering by quotient affinity
- [ ] Stress-based placement of siblings (distance ∝ 1/affinity) on the small quotient graphs, replacing the force solve
- [ ] Redesign or drop the "pull towards external neighbors" term (no measurable gain in the lab: ratio 0.226 with, 0.223 without)
- [ ] Parallelize the serial apply pass (layout scales only 6–7× at 32 threads)

**Metrics.** Occupied area fraction; median edge length ÷ median random-pair distance (lab: 0.014); neighborhood preservation at each level; layout time and memory on lj/orkut; byte-identical output across thread counts.

**Done when.** A recommended layout with images and numbers on lj and orkut, and a go/no-go on replacing disc packing in {{#m5-01-layout}}.

**Pointers.** `lab/e-map`, FINDINGS § E; literature: GMap, Voronoi treemaps, FM3/sfdp multilevel.
