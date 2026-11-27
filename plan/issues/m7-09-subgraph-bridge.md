---
title: Subgraph extraction to small-graph tools
milestone: M7 · Queries and views
labels: type:feature, area:python, P1, risk:low, size:S
---
**Why.** The common workflow at scale is "find the interesting 5,000 nodes, then study them with familiar tools". Everygraph should make the hand-off trivial instead of reimplementing every small-graph algorithm.

**Scope**
- [ ] `g.subgraph(nodes)` → a `View`, for any node set: ids, a community `c[i]`, a k-hop ball `g.khop(…)`, a map selection (`docs/api-sketch.md` § 3.3). The planner materializes it when it is small; it keeps properties and original ids (`docs/api-sketch.md` § 7: a `View`, not a small `Graph`)
- [ ] One-call conversion to NetworkX / igraph / a Polars edge table, with layout coordinates from the map attached. The NetworkX and igraph converters are {{#m6-05-import-export}}'s (`to_networkx()`, `to_igraph()`, decided 2026-11-26); this issue calls them on the extracted subgraph and adds the coordinates
- [ ] Size guards and sampling options for neighbourhoods that explode (hubs)

**Acceptance**
- Ego network (2 hops, `limit=10_000` nodes) of a node in Friendster → NetworkX graph in < 100 ms in RAM

**Depends on** {{#m6-05-import-export}}, {{#m6-01-python-api}}, {{#m7-01-point-queries}}, {{#m7-04-views}}, {{#m5-01-layout}}
