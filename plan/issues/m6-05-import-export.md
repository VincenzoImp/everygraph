---
title: Import and export: NetworkX, igraph, GraphML, GEXF, Parquet, Matrix Market
milestone: M6 · Python and interop
labels: type:feature, area:interop, P1, risk:low, size:M
---
**Why.** Competitors are also channels: people arrive from Gephi and NetworkX when their graph gets too big, and leave to them with a small extract. Moving both ways must be one call.

**Scope**
- [ ] Readers: GraphML, GEXF, Matrix Market, edge-list dialects (SNAP, KONECT), node/edge CSV pairs (Neo4j `neo4j-admin` export layout)
- [ ] Writers: Parquet/CSV node and edge tables, GraphML, GEXF (with layout coordinates and community as attributes so Gephi opens our map as a starting layout), Matrix Market
- [ ] Converters: `to_networkx()`, `to_igraph()`, `to_scipy()`, with a size guard and a clear error above it
- [ ] Kept loops: `to_networkx()` and `to_igraph()` keep them as loop edges, one edge per pair with its `edges` and summed weight; `to_scipy()` puts w, not 2w, on the diagonal (the `networkx.to_scipy_sparse_array` convention, so the matrix round-trips) and never refuses a weight; above `max_edges` is `Unsupported` (2) (`docs/spec/graph-semantics.md` § 6, § 10 item 9)
- [ ] Apache GraphAr as an interchange format: evaluate once it has a stable release (still incubating; last release 0.13.0)
- [ ] Round-trip tests for properties and ids

**Acceptance**
- A community of 10^4 nodes exported to GEXF opens in Gephi with positions and colours
- Importing a Neo4j CSV export of 100 M relationships works within the ingest performance envelope

**Depends on** {{#m6-01-python-api}}, {{#m5-01-layout}}
