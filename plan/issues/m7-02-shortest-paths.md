---
title: Shortest paths between nodes
milestone: M7 · Queries and views
labels: type:feature, area:query, P0, risk:medium, size:M, lab-evidence
---
**Why.** "How are these two connected?" is the investigator's core question and the weakest point of embedded engines today (0.3–4.8 s on lj; Neo4j 6.4 ms). The lab's bidirectional BFS answers in 0.3–1.5 ms in RAM.

**Caveat.** The 0.3–1.5 ms figure is an in-process kernel on uncompressed arrays; the Neo4j and embedded-engine figures were measured through their query interfaces. Report both levels (kernel and end-to-end API call) in our own benchmarks.

**Scope**
- [ ] Bidirectional BFS for unweighted paths; on directed graphs the target side expands in-edges ({{#m2-11-in-adjacency}})
- [ ] Weighted: bidirectional Dijkstra on a per-entry weight column
- [ ] Results: one shortest path, all shortest paths (bounded), k shortest (stretch); path as node ids with edge properties
- [ ] Constraints: maximum hops, node/edge filters (a view), avoid-set; these are what make paths useful in investigations (e.g. "not through the exchange hub")
- [ ] Budgets and timeouts — tails reach 113 ms p99 on twitter-2010 because of hubs
- [ ] Batch pairs API

**Acceptance**
- Verified against plain BFS on 1,000 random pairs per tier-2 graph
- p50 < 2 ms and p95 < 20 ms in RAM on twitter-2010, single thread
- A constrained query (max 4 hops, avoid top-100 hubs) works on Friendster
- Among several shortest paths the one returned, and the order of the alternatives, follow a documented rule and are the same at any thread count, regime and budget (tested). A step reports the earliest edge of its pair inside the view — for a weighted path the cheapest, earliest among equal costs — and the pair's number of edges (`docs/api-sketch.md` § 2.12)

**Depends on** {{#m7-01-point-queries}}
