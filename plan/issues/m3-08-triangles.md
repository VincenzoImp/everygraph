---
title: Triangle counting and clustering coefficients
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P1, risk:low, size:S, lab-evidence
---
**Scope**
- [ ] Degree-ordered forward adjacency with merge intersection (lab, idle: orkut 9.24 s at 4 threads → 2.51 s at 16 → 1.46 s at 32, the one kernel that clearly benefits from SMT; Friendster 57.5 s on a loaded machine)
- [ ] Global count, per-node counts, local clustering coefficient, transitivity
- [ ] Kept loops are ignored: not a triangle, and not in the degree of the coefficients: `c(v) = 2·T(v)/(k'(k' − 1))` with `k' = |N(v) \ {v}|` (degree − 2 at a looped node), 0 when k' < 2; `τ = Σ T(v) / Σ k'(k' − 1)/2`, 0 without a connected triple (`docs/spec/graph-semantics.md` § 5.3, § 5.4, S3, S4)
- [ ] LCC of a directed graph by the LDBC Graphalytics definition (the neighbourhood is the union of in- and out-neighbours; the count is of arcs among them), on the in-adjacency ({{#m2-11-in-adjacency}}); needed by {{#m8-11-graphalytics-eligibility}}. Out of core it uses the same MGT-style bounded-memory path as below, over the union neighbourhood (added 2026-11-26, `docs/research/sota-review-2026-11.md` § 7: LCC out of core is a weak point for graph500-30)
- [ ] SIMD intersection for long lists; galloping for skewed pairs
- [ ] Memory: the forward adjacency is a derived artifact — declare it to the planner; out-of-core behaviour documented (intersection is random-access heavy)
- [ ] A bounded-memory path: MGT-style chunked listing (Hu, Tao, Chung, SIGMOD 2013; exact, Θ(m/M) passes over the oriented graph), with PDTL's parallel split; per-node counts in O(n). Approximate wedge sampling only as an explicitly labelled variant (`docs/research/memory-bounded-algorithms.md`)

**Acceptance**
- Counts equal the known values: lj 177,820,130; orkut 627,584,181; Friendster 4,173,724,142 (verify these against published figures, the lab did not look them up)
- Per-node values equal NetworkX on oracle graphs
- Triangles, clustering and transitivity are the same with loops kept and dropped on every oracle graph (`docs/spec/graph-semantics.md` § 5.3, § 5.4)
- A like-for-like reference timing is added to the benchmarks (the first lab compared against NetworKit's local clustering coefficient in non-turbo mode, which is not a plain triangle count)

**Depends on** {{#m3-01-edge-map}}, {{#m2-11-in-adjacency}} (the directed LCC only)
