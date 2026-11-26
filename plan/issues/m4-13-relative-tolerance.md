---
title: Leiden: a stopping tolerance that scales with the graph's structure
milestone: M4 · Communities and hierarchy
labels: type:experiment, area:community, P2, risk:medium, size:S
---
**Why.** Local moving stops when a sweep gains at most `0.01` of modularity on level 0, ten times less per level and per iteration (the lab's rule, kept by {{#m4-01-leiden-production}}). On a low-modularity graph that stops too early. `planted-20x100-80-8` (two thirds of each node's edges leave its block; `crates/everygraph-community/tests/leiden.rs`) gives 0.2714 at the default and 0.2914 in the quality mode (deterministic; the test pins them with a small margin since PR #287), against 0.2909 for the best of igraph Leiden, igraph multilevel and NetworkX Louvain (`tests/references.json`). The fast mode moves with the schedule: 0.2422–0.2849 in local runs, down to 0.216 on loaded and arm64 CI runners, so its gate there is 0.19 (PR #299). On the values before PR #287, a first tolerance of `1e-4` raised all three modes, but took about 1.9× as long on lj (exploratory, PR #232).

**Scope**
- [ ] Candidates: a tolerance relative to the level's first-sweep gain, or to the modularity at the start of the level; today's absolute rule as the baseline.
- [ ] Interleaved A/B on the six graph families of {{#m1-10-graph-families}} and the planted graphs of the test: modularity, sweeps, time; deterministic hash identity at 1, 4, 16 threads.
- [ ] If a candidate wins, the test's pinned values and gates for `planted-20x100-80-8` (`modularity_is_within_the_gates_of_networkx_and_igraph`) change with it.

**Done when.** The rule is chosen on the six families, not on lj alone, and the result is in FINDINGS.

**Depends on** {{#m4-01-leiden-production}}, {{#m1-10-graph-families}}
