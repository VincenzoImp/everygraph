---
title: Datasets: a real bipartite graph and a real co-occurrence graph with terms anyone can use
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P2, risk:low, size:S
---
**Why.** The registry ({{#m0-04-datasets}}) covers the bipartite family only with a generated graph, and the weighted family with a generated graph plus a Wikipedia clickstream (click counts between articles, CC0), which is not a co-occurrence graph. MovieLens 100K was dropped because its terms forbid commercial use: tier 1 holds only data anyone may use. Kernels that behave differently on these shapes (projections, weighted modularity, k-core on bipartite graphs) deserve one real graph each.

**Scope**
- [ ] One real bipartite graph (two node sets, at least 10^5 edges) whose terms are stated by the source and allow any use (CC0, CC BY, public domain, ODbL with attribution), with a stable URL — candidates to check: Wikimedia analytics datasets, open government data, OpenAlex author–work extracts
- [ ] One real positively weighted co-occurrence graph under the same conditions
- [ ] Registry entries with pinned checksums, counts checked on the file, terms with a link; tier 1 if under ~30 MB, otherwise tier 2

**Acceptance**
- `everygraph-bench fetch` verifies both; the family tests of the registry pass with the real graphs standing for their family
- If no source with clear terms exists, the issue is closed with the list of sources checked and why each was rejected

**Depends on** {{#m0-04-datasets}}
