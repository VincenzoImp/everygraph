---
title: Communities and their levels in the Python API: Arrow tables and community views
milestone: M6 · Python and interop
labels: type:feature, area:python, P1, risk:low, size:M
---
**Why.** {{#m4-06-hierarchy-artifact}} stores every level of a community result in the graph file and reads it back in Rust as columns (`everygraph_community::hierarchy::Hierarchy`). The user-facing objects of `docs/api-sketch.md` § 3.5 are not built on it yet: there is no Arrow table and no view of a community.

**Scope**
- [ ] `c.table(level)` and `c.nodes(level)` as Arrow tables with the columns of `docs/api-sketch.md` § 3.4 (`community`, `nodes`, `internal_edges`, `external_edges`, `share`, `parent`), built zero-copy from the stored columns ({{#m6-03-arrow-interop}}).
- [ ] `c.of(nodes)` (level, community, nodes, from level 1 down) and `c.levels`, from the stored hierarchy.
- [ ] `c[i]` usable wherever a node set is expected: `g.subgraph(c[i])` as a `View` ({{#m7-09-subgraph-bridge}}), with its edge count known from the table for the size guard.

**Done when.** The tables equal the stored hierarchy column for column on the oracle graphs; reading a table of 1 M communities stays under 50 ms (the target of {{#m4-06-hierarchy-artifact}}), measured cold and warm; `g.subgraph(c[i])` has the community's nodes and internal edges.

**Depends on** {{#m4-06-hierarchy-artifact}}, {{#m6-01-python-api}}, {{#m6-03-arrow-interop}}
