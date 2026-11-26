---
title: Leiden: a stopping tolerance that scales with the graph's structure
milestone: M4 · Communities and hierarchy
labels: type:experiment, area:community, P2, risk:medium, size:S
---
**Why.** Local moving stops when a sweep gains at most `0.01` of modularity on level 0, ten times less per level and per iteration (the lab's rule, kept by {{#m4-01-leiden-production}}). On a low-modularity graph that stops too early. `planted-20x100-80-8` (two thirds of each node's edges leave its block; `crates/everygraph-community/tests/leiden.rs`) gives 0.2602 at the default, 0.2879 in the quality mode and 0.237–0.258 fast, against 0.2909 for the best of igraph Leiden, igraph multilevel and NetworkX Louvain (`tests/references.json`). A first tolerance of `1e-4` gives 0.2824, 0.2881, 0.2765, but takes about 1.9× as long on lj (exploratory, PR #232).

**Scope**
- [ ] Candidates: a tolerance relative to the level's first-sweep gain, or to the modularity at the start of the level; today's absolute rule as the baseline.
- [ ] Interleaved A/B on the six graph families of {{#m1-10-graph-families}} and the planted graphs of the test: modularity, sweeps, time; deterministic hash identity at 1, 4, 16 threads.
- [ ] If a candidate wins, the test's pinned values for `planted-20x100-80-8` change with it.

**Done when.** The rule is chosen on the six families, not on lj alone, and the result is in FINDINGS.

**Depends on** {{#m4-01-leiden-production}}, {{#m1-10-graph-families}}
