---
title: Subgraph extraction to small-graph tools
milestone: M6 · Python and interop
labels: type:feature, area:python, P1, risk:low, size:S
---
**Why.** The common workflow at scale is "find the interesting 5,000 nodes, then study them with familiar tools". Everygraph should make the hand-off trivial instead of reimplementing every small-graph algorithm.

**Scope**
- [ ] `g.subgraph(nodes | community | k-hop ball | view)` → a small `Graph` (in memory) with properties and original ids
- [ ] One-call conversion to NetworkX / igraph / a Polars edge table, with layout coordinates from the map attached
- [ ] Size guards and sampling options for neighbourhoods that explode (hubs)

**Acceptance**
- Ego network (2 hops, budget 10k nodes) of a node in Friendster → NetworkX graph in < 100 ms in RAM
