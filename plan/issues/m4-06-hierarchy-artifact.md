---
title: Hierarchy artifact with exact aggregates
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P0, risk:low, size:M, lab-evidence
---
**Why.** The map, the per-community tables and the "exact numbers at every level" promise all read this artifact. The lab format was verified independently (node counts sum to n, volumes to 2m, internal + cut edges to m, quotient row sums equal volumes) and 1,403,208 lj communities matched between two independent implementations.

**Scope**
- [ ] Per level k: parent map, node count, degree volume, internal edge count (or weight), cut weight; quotient graph in the standard adjacency format with f64 weights
- [ ] Stored as a derived artifact with provenance ({{#m2-12-artifact-cache}})
- [ ] Conservation checks run on write and in `eg verify`
- [ ] API: membership at any level as a column, community table as Arrow, `subgraph(community)` as a view, ancestors/descendants of a community
- [ ] Per-community summary statistics of node properties (counts by category, top-k by PageRank) for map labels

**Acceptance**
- Invariants hold on every tier-1/2 graph and every level
- Reading a community table of 1 M communities takes < 50 ms

**Pointers.** `--out-hier` in `lab/b2-leiden`; `/data/hier` format in LAB.md.
