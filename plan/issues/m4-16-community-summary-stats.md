---
title: Per-community summary statistics of node properties, for map labels
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:low, size:M
---
**Why.** A map label names a region by what is in it: the most common categories of its nodes and its most central nodes. {{#m4-06-hierarchy-artifact}} stores the exact counts of every community at every level, but not these summaries. The build stores the property columns they read ({{#m2-34-build-columns}}).

**Scope**
- [ ] For each community of a level of a stored hierarchy: counts by category of a categorical node column (the top k categories and an "other" count), and the top k nodes by a score column (PageRank by default), ties in graph order.
- [ ] Computed in one pass over the membership and the columns, in memory linear in the communities times k, and stored as parts of the hierarchy result (`docs/spec/eg-communities.md`) or as a result of their own keyed by the hierarchy's key and the columns.
- [ ] Read by the map's labels ({{#m4-07-map-hierarchy}}, {{#m5-01-layout}}) and the community table.

**Done when.** On a graph with a categorical column, the counts of every community at every level add up to the counts of the level above and of the whole column, an independent recount in the test agrees, and `everygraph verify --deep` checks them.

**Depends on** {{#m4-06-hierarchy-artifact}}, {{#m2-34-build-columns}}
