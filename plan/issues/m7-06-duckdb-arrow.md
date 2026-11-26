---
title: DuckDB and SQL access through Arrow
milestone: M7 · Queries and views
labels: type:feature, area:interop, P1, risk:medium, size:M, lab-evidence
---
**Why.** DuckDB is the centre of gravity for local analytics. Rather than compete on SQL, make graph results first-class tables there. Note the state of the alternative: the DuckPGQ extension did not load on DuckDB 2.0.1 in the lab, and its path queries timed out or ran out of memory on lj.

**Scope**
- [ ] Python path first: node, edge and result tables registered in DuckDB via Arrow with no copy; documented patterns for joining labels and aggregating by community
- [ ] `.eg` property and result sections readable as Arrow/Parquet-like tables without the engine (ties to {{#m2-10-property-columns}})
- [ ] A DuckDB extension exposing table functions (`eg_neighbors`, `eg_shortest_path`, `eg_communities`, `eg_pagerank`) — scoped as a prototype; the full extension is tracked after v0.1 if the Python path proves the demand
- [ ] SQL-shaped ingest: a tested recipe for `eg.read(duckdb_relation)`; the import itself is {{#m6-03-arrow-interop}}'s (any Arrow PyCapsule object)

**Acceptance**
- The tutorial query "total volume per community joined with my labels" runs in DuckDB over Everygraph tables without materializing a copy
- A written go/no-go on a native DuckDB extension with an effort estimate

**Depends on** {{#m6-03-arrow-interop}}
