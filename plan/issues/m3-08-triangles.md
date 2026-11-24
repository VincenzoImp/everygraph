---
title: Triangle counting and clustering coefficients
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P1, risk:low, size:S, lab-evidence
---
**Scope**
- [ ] Degree-ordered forward adjacency with merge intersection (lab: orkut 1.46 s at 32 threads, the best-scaling kernel at 13.7× from 1 to 32; Friendster 57.5 s)
- [ ] Global count, per-node counts, local clustering coefficient, transitivity
- [ ] SIMD intersection for long lists; galloping for skewed pairs
- [ ] Memory: the forward adjacency is a derived artifact — declare it to the planner; out-of-core behaviour documented (intersection is random-access heavy)

**Acceptance**
- Counts equal the known values: lj 177,820,130; orkut 627,584,181; Friendster 4,173,724,142 (verify these against published figures, the lab did not look them up)
- Per-node values equal NetworkX on oracle graphs

**Depends on** {{#m3-01-edge-map}}
