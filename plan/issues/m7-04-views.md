---
title: Filtered views: mask or materialize
milestone: M7 · Queries and views
labels: type:feature, area:query, P0, risk:medium, size:L, lab-evidence
---
**Why.** "Only edges above 10 ETH", "only this community", "only 2024" must compose with every kernel and query. The lab changed the plan here: **a mask never clearly beats the unfiltered scan** (0.91–2.09× per PageRank iteration; the kernel still visits every entry), while a materialized filtered CSR costs 47–135 ms to build on orkut — about 1–2.5 unfiltered PageRank iterations (DESIGN D10) — and then runs PageRank at 0.09–0.17× the time for 1–10% selectivity (orkut, 32 threads; 0.09× at 4 threads).

**Scope**
- [ ] View definition: predicates on node and edge properties, node sets, community membership, time ranges; composable; stored by definition (cheap) and optionally materialized
- [ ] Edge predicates on per-event columns (time, each row's weight) select events through the event offsets, and the view recomputes each pair's multiplicity and sums from the events it keeps (ADR 0028; `docs/spec/graph-semantics.md` § 6)
- [ ] A materialized view reduces like its mask (D14; `design/DECISIONS.md`, "Masked and materialized views give the same bits"). It renumbers its nodes densely, stores the map from its new ids to the parent's ids, and runs every result-bearing reduction over the parent graph's node blocks (ADR 0028, A2), looked up through that map. A view saved with `save()` keeps the map, so it reduces the same way when opened again
- [ ] Mask representation: plain bitset over events, entries or nodes (3% of topology bytes for an entry bitset); Roaring only below ~5% density or for persisted views (same size as a bitset at ≥ 10% random density)
- [ ] Parallel mask build (lab: ~10 G entries/s for a time predicate)
- [ ] Materialization into the standard adjacency format, cached as a derived artifact
- [ ] Planner rule derived from measurement ({{#m3-02-planner}}): on orkut and twitter-sym, idle machine, 16 threads, at 1, 5, 10, 25, 50 and 90% selectivity, measure PageRank (20 iterations), BFS and components on a mask and on a materialized view, including build time and memory, and write the rule from the crossover. The lab found the materialized view faster per PageRank iteration at every selectivity it measured (1–90%; no point between 10% and 50%), the mask ahead only for few-pass work and on memory — so "~25%" was never a measurement (ADR 0010, D10 open)
- [ ] Views that skip work (added 2026-11-25, `docs/research/sota-ingest-query.md` § 3 D10, § 4 C5; `docs/research/README.md` B18). The lab's mask was an *entry* bitset that still visits every entry. Add two arms to the crossover grid above (orkut, twitter-sym, 16 threads, 1–90%):
  - a **node mask that skips whole lists** through the per-node Elias–Fano offsets (D4);
  - a **time or weight predicate with block zone maps**: per-block min/max, so a window or `value > x` skips whole ~64K-edge blocks. This issue specifies and builds them (moved from {{#m2-10-property-columns}}): "Per-block min/max of time and weight columns (zone maps aligned to the ~64K-edge adjacency blocks), as one optional section", a container kind of its own beside the columns of `docs/spec/eg-columns.md`.

  Results must stay identical to the materialized view: both reduce over the parent graph's node blocks (D14, the scope item above). graph-tool's mask views ("no overhead when not filtered") and Kùzu's semijoin filters are the precedent
- [ ] Branchless masked kernels (50% predicates are the worst case for branch prediction — a lab todo)
- [ ] Induced subgraph vs edge-filtered semantics spelled out

**Acceptance**
- PageRank on a 5% edge view of orkut ≥ 5× faster than on the full graph, including materialization amortized over 20 iterations
- Any kernel and any point query accepts a view; oracle tests compare against NetworkX subgraphs
- A deterministic kernel gives the identical result on a view whether the planner runs it as a mask or materialized (a materialized view keeps graph order, so ties break the same way, and reduces over the parent graph's node blocks through its map to parent ids, so floats sum in the same order): tested for PageRank, components and communities (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises"); the same holds for that view saved with `save()` and opened again
- On orkut, for a 10% edge view and a 10% node view, PageRank f64 run on the mask and on the materialized view gives byte-identical ranks at 1 and 16 threads (the materialized view reducing over the parent's node blocks)
- A view's `num_nodes` and `num_edges` are computed by one planned pass on first use and never by printing the view (`docs/api-sketch.md` § 3.3)

**Pointers.** `lab/f1-query/src/bin/props.rs`.

**Depends on** {{#m2-10-property-columns}}, {{#m2-34-build-columns}} (no graph carries a property to filter on before it), {{#m3-01-edge-map}}
