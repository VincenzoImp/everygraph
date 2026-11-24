---
title: A hierarchy the map can use: bounded fan-out without losing meaning
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P0, risk:high, size:L, lab-evidence
---
**Question.** How do we get zoom levels with fan-out roughly 8–64 whose regions mean something to the user?

**What the lab found.** Natural Leiden hierarchies shrink 3.5–18× per level but have parents with up to ~100,000 children. A fan-out cap of 64 with level compaction bounds fan-out on all but the top one or two levels, at a cost of 2–5 modularity points, and the median fan-out is only 2–3. The top levels are star-shaped: a few giants each surrounded by hundreds or thousands of small satellite communities. The layout then had to invent "derived regions" (the many identical "627 nodes" bubbles), which look like communities and are not.

**Candidates**
- [ ] Top-down recursive sub-clustering of oversized communities on the exported quotient graphs (not tested in the lab)
- [ ] Nested resolution levels: run CPM/modularity at increasing γ *inside* each parent so levels nest by construction (plain γ sweeps are not nested)
- [ ] Satellite handling: an explicit "N small communities" aggregate per giant instead of fake regions
- [ ] Keep the cap approach as baseline; fix the known bug (`--cap` ignored after the first iteration)

**Metrics**
- Fan-out distribution per level (median, p95, max, share in 4–64)
- Modularity of each level vs the uncapped partition; conductance of regions
- Share of nodes living in "derived" (non-community) regions
- Stability: does a small perturbation of the graph reshuffle the top levels?
- Visual check on lj, orkut and one non-social graph through the lab layout

**Decision it feeds.** D16 and the artifact format in {{#m4-07-map-hierarchy}}.

**Done when.** One method is chosen with numbers on three graphs, and the map screenshots no longer contain unlabeled artificial regions.
