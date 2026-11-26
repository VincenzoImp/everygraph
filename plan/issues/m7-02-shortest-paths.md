---
title: Shortest paths between nodes
milestone: M7 · Queries and views
labels: type:feature, area:query, P0, risk:medium, size:M, lab-evidence
---
**Why.** "How are these two connected?" is the investigator's core question and the weakest point of embedded engines today (0.3–4.8 s on lj; Neo4j 6.4 ms). The lab's bidirectional BFS answers in 0.13–0.47 ms p50 in RAM on raw arrays and 0.16–0.56 ms on the default storage (#118, idle, single thread, lj and twitter-2010 directed).

**Caveat.** The first lab's 0.3–1.5 ms came from a shared machine; idle (#118) 0.13–0.47 ms raw, 0.16–0.56 ms default storage, lj and twitter-2010, p50. These are in-process kernels; the Neo4j and embedded-engine figures were measured through their query interfaces. Report both levels (kernel and end-to-end API call) in our own benchmarks.

**Scope**
- [ ] Bidirectional BFS for unweighted paths; on directed graphs the target side expands in-edges ({{#m2-11-in-adjacency}})
- [ ] Weighted: bidirectional Dijkstra on a per-pair cost column (kernels read pairs, never per-event columns, ADR 0028; which pair aggregates are stored is {{#m2-10-property-columns}}'s decision)
- [ ] A step's cost is the smallest non-null event cost of its pair; under `duplicates="sum"` it is the stored sum, under `"collapse"` the kept row's value; a pair whose costs are all null is not used; a kept loop is never a step; a negative, NaN or ∞ cost is `InvalidInput` (3) (`docs/spec/graph-semantics.md` § 4.2, § 6)
- [ ] Results: one shortest path, all shortest paths (bounded), k shortest (stretch); path as node ids with edge properties
- [ ] Constraints: maximum hops, node/edge filters (a view), avoid-set; these are what make paths useful in investigations (e.g. "not through the exchange hub")
- [ ] Budgets and timeouts — tails reach 113 ms p99 on twitter-2010 because of hubs (shared machine; idle, #118: 32.3 ms raw, 27.3 ms default storage, graph in heap)
- [ ] Batch pairs API
- [ ] A parent column for single-source SSSP, as BFS has (moved from {{#m3-06-traversal}}, whose Δ*-stepping kernel returns distances only), and its tie rule added to `docs/api-sketch.md` § 2.12

**Acceptance**
- Verified against plain BFS on 1,000 random pairs per tier-2 graph
- p50 < 2 ms and p95 < 20 ms in RAM on twitter-2010, single thread
- A constrained query (max 4 hops, avoid top-100 hubs) works on Friendster
- Among several shortest paths the one returned, and the order of the alternatives, follow a documented rule and are the same at any thread count, regime and budget (tested). A step reports the earliest edge of its pair inside the view — for a weighted path the cheapest, earliest among equal costs — and the pair's number of edges (`docs/api-sketch.md` § 2.12)

**Depends on** {{#m7-01-point-queries}}
