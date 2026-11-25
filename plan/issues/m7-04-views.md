---
title: Filtered views: mask or materialize
milestone: M7 · Queries and views
labels: type:feature, area:query, P0, risk:medium, size:L, lab-evidence
---
**Why.** "Only edges above 10 ETH", "only this community", "only 2024" must compose with every kernel and query. The lab changed the plan here: **a mask never beats the unfiltered scan** (the kernel still visits every entry), while a materialized filtered CSR costs 47–135 ms to build on orkut — one or two PageRank iterations — and then runs PageRank at 0.09–0.17× the time for 1–10% selectivity (orkut, 32 threads; 0.09× at 4 threads).

**Scope**
- [ ] View definition: predicates on node and edge properties, node sets, community membership, time ranges; composable; stored by definition (cheap) and optionally materialized
- [ ] Mask representation: plain bitset over entries or nodes (3% of topology bytes); Roaring only below ~5% density or for persisted views (same size as a bitset at ≥ 10% random density)
- [ ] Parallel mask build (lab: ~10 G entries/s for a time predicate)
- [ ] Materialization into the standard adjacency format, cached as a derived artifact
- [ ] Planner rule derived from measurement ({{#m3-02-planner}}): on orkut and twitter-sym, idle machine, 16 threads, at 1, 5, 10, 25, 50 and 90% selectivity, measure PageRank (20 iterations), BFS and components on a mask and on a materialized view, including build time and memory, and write the rule from the crossover. The lab found the materialized view faster per PageRank iteration at every selectivity it measured (1–90%; no point between 10% and 50%), the mask ahead only for few-pass work and on memory — so "~25%" was never a measurement (ADR 0010, D10 open)
- [ ] Views that skip work (added 2026-11-25, `docs/research/sota-ingest-query.md` § 3 D10, § 4 C5; `docs/research/README.md` B18). The lab's mask was an *entry* bitset that still visits every entry. Add two arms to the crossover grid above (orkut, twitter-sym, 16 threads, 1–90%):
  - a **node mask that skips whole lists** through the per-node Elias–Fano offsets (D4);
  - a **time or weight predicate with block zone maps**: per-block min/max ({{#m2-10-property-columns}}), so a window or `value > x` skips whole ~64K-edge blocks.

  Results must stay identical to the materialized view, so the summation order must not change (D14). graph-tool's mask views ("no overhead when not filtered") and Kùzu's semijoin filters are the precedent
- [ ] Branchless masked kernels (50% predicates are the worst case for branch prediction — a lab todo)
- [ ] Induced subgraph vs edge-filtered semantics spelled out

**Acceptance**
- PageRank on a 5% edge view of orkut ≥ 5× faster than on the full graph, including materialization amortized over 20 iterations
- Any kernel and any point query accepts a view; oracle tests compare against NetworkX subgraphs
- A deterministic kernel gives the identical result on a view whether the planner runs it as a mask or materialized (node order of a materialized view preserves graph order, so tie-breaks and summation order do not change): tested for PageRank, components and communities (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises"); the same holds for that view saved with `save()` and opened again
- A view's `num_nodes` and `num_edges` are computed by one planned pass on first use and never by printing the view (`docs/api-sketch.md` § 3.3)

**Pointers.** `lab/f1-query/src/bin/props.rs`.

**Depends on** {{#m2-10-property-columns}}, {{#m3-01-edge-map}}
