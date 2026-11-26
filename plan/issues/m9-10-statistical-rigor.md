---
title: Statistical rigor: significance of communities and SBM on the coarsened graph
milestone: M9 · After v0.1
labels: type:experiment, area:community, P2, risk:medium, size:L
---
**Why.** Modularity maximization finds communities in random graphs and has a resolution limit. Researchers who need statistical evidence use stochastic block models (graph-tool), which do not scale: the research pass found a fastest SBM variant taking 38.7 hours on a 14 M-node graph, single core. Running inference on our coarsened hierarchy could offer rigor at scale.

**Scope**
- [ ] Null-model comparison: modularity of the found partition vs degree-preserving randomizations, per level
- [ ] Per-community significance scores against the configuration model
- [ ] Export the level-k quotient graph to graph-tool for nested SBM; compare its hierarchy with ours
- [ ] Consensus/stability across seeds as an uncertainty measure shown on the map

**Done when.** A documented recipe "when you need statistical evidence" and a decision on native support.

**Depends on** {{#m4-06-hierarchy-artifact}}, {{#m4-08-community-tools}}
