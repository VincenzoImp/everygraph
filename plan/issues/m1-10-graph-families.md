---
title: Beyond social networks: web, road, bipartite, weighted and directed graphs
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:bench, P0, risk:medium, size:L
---
**Question.** Every quality and layout conclusion so far rests on two undirected, unweighted social graphs. Do they hold elsewhere?

**Scope**
- [ ] Families: web (uk-2005, and the Web Data Commons 2012 host graph: 101 M nodes, 2,043 M arcs, real web topology, Common Crawl terms of use; `docs/research/memory-bounded-algorithms.md` addendum item 18), road (europe_osm — high diameter, degree ≈ 2), bipartite (user–item), weighted (e.g. a co-occurrence graph), directed social (twitter-2010), a transaction-style multigraph with timestamps
- [ ] Per family: ingest rate and bits/edge; kernel times and scaling (BFS on high-diameter graphs is a known weak spot for direction-optimizing BFS); Leiden modularity vs the best reference and number of iterations; hierarchy fan-out; a layout image
- [ ] Directed graphs: decide the community objective (symmetrize vs directed modularity) and what PageRank/WCC/SCC mean in the API
- [ ] Bipartite: does the map make sense, or does it need a projection?

**Decision it feeds.** Quality gates in {{#m4-05-quality-gates}} ("six graph families") and default behaviours for directed and weighted inputs.

**Done when.** A per-family table in FINDINGS and a list of family-specific defects filed as issues.

**Depends on** {{#m0-04-datasets}}
