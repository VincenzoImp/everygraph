---
title: Filtered views: mask or materialize
milestone: M7 · Queries and views
labels: type:feature, area:query, P0, risk:medium, size:L, lab-evidence
---
**Why.** "Only edges above 10 ETH", "only this community", "only 2024" must compose with every kernel and query. The lab changed the plan here: **a mask never beats the unfiltered scan** (the kernel still visits every entry), while a materialized filtered CSR costs 47–135 ms to build on orkut — one or two PageRank iterations — and then runs PageRank at 0.09× the time for 1–10% selectivity.

**Scope**
- [ ] View definition: predicates on node and edge properties, node sets, community membership, time ranges; composable; stored by definition (cheap) and optionally materialized
- [ ] Mask representation: plain bitset over entries or nodes (3% of topology bytes); Roaring only below ~5% density or for persisted views (same size as a bitset at ≥ 10% random density)
- [ ] Parallel mask build (lab: ~10 G entries/s for a time predicate)
- [ ] Materialization into the standard adjacency format, cached as a derived artifact
- [ ] Planner rule: materialize when selectivity < ~25% and the view will be iterated; otherwise mask ({{#m3-02-planner}})
- [ ] Branchless masked kernels (50% predicates are the worst case for branch prediction — a lab todo)
- [ ] Induced subgraph vs edge-filtered semantics spelled out

**Acceptance**
- PageRank on a 5% edge view of orkut ≥ 5× faster than on the full graph, including materialization amortized over 20 iterations
- Any kernel and any point query accepts a view; oracle tests compare against NetworkX subgraphs
- A deterministic kernel gives the identical result on a view whether the planner runs it as a mask or materialized (node order of a materialized view preserves graph order, so tie-breaks and summation order do not change): tested for PageRank, components and communities (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises")

**Pointers.** `lab/f1-query/src/bin/props.rs`.

**Depends on** {{#m2-10-property-columns}}, {{#m3-01-edge-map}}
