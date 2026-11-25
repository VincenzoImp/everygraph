---
title: Oracle: check every graph-semantics option, kept self-loops included
milestone: M2 · Format and ingest
labels: type:infra, area:engine, P0, risk:medium, size:M
---
**Why.** The correctness oracle ({{#m0-05-oracle}}) leaves PageRank, core numbers, clustering, transitivity and modularity unchecked on graphs with kept self-loops, and has no `duplicates` option, because the semantics were not decided. They now are (`docs/spec/graph-semantics.md`, §5 and §6; `design/DECISIONS.md`, "Graph semantics, the cases left to #36"). Every kernel built in M3 and M4 is tested against these references, so they must implement the decided semantics first.

**Scope** (the exact list is in PR #145's final comment, section E)
- [ ] `oracle/SEMANTICS.md`: Q2 decided; kept-loop table rows with the decided values; the quotient carries loops; the `duplicates` rule
- [ ] Rust references (`crates/everygraph-oracle`): PageRank with A[v][v] = 2w undirected, w directed; modularity and quotient with loops; core numbers, clustering, transitivity ignore loops; adjacency output lists v once when the loop is kept; per-node loop weight sums in the projection; `duplicates: Events | Collapse | Sum` with the ingest report and its identity
- [ ] Python references (`oracle/src`): the same, mapped onto NetworkX and igraph conventions (an `ADJUSTMENTS` entry for each place a library differs); probe rows for the decided cases (lone-loop coreness, weighted loop w = 3: PageRank [38/63, 25/126, 25/126], modularity −1/18, clustering and transitivity with a pendant, directed modularity with a loop)
- [ ] Invariants: no loop unless kept, at most one per node; modularity(partition) = modularity(quotient, singletons); PageRank sums to 1 with loops; core, clustering and transitivity equal with loops kept and dropped
- [ ] `corpus.rs`: no kernel excluded on kept-loop graphs; generators for each `duplicates` mode

**Acceptance**
- The CI and nightly tiers pass with every kernel checked on kept-loop graphs and under each `duplicates` mode; a seeded bug that counts an undirected loop once in PageRank is caught by at least two families

**Depends on** {{#m0-05-oracle}}, {{#m2-09-graph-semantics}}
