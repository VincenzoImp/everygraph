---
title: Layout baseline: our map against a published large-graph layout
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:map, P1, risk:medium, size:S, lab-evidence
---
**Question.** D17's evidence that hierarchy-constrained disc packing "reads better" compares it with an 8-dimensional FastRP + PCA layout, a blob. That is not a published large-graph layout. On the mean edge ratio the gap is only 0.226 against 0.264 (lj, `lab/e-map/results/eval_lj_hcl.json`), and the p90 edge length (0.430) equals the random-pair median (0.427). How does our layout compare with one published large-graph layout on the same graph, on metrics that do not depend on our own hierarchy?

**Candidates** (`docs/research/sota-maps.md` § 2, challenge 3; paper numbers, their machines). At least one of these is run, more where they fit:
- DRGraph: public code; ran com-Orkut in 18.4 GB on one thread (i7-6700).
- sfdp: Graphviz, EPL-2.0.
- OGDF FM³: the classical stress winners on large graphs.
- BatchLayout: states the "same layout irrespective of number of threads" (§ 4.5).

Each is a reference tool run outside the product, never a dependency.

**Method**
- [ ] One lab script; one run per tool on lj, and on orkut where it fits. Each run under `systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0`, raised up to 24G for a tool that needs it, the limit stated with each run. Record what each tool actually ran: version, options, iterations, threads
- [ ] Metrics, the same script for every layout, ours included:
  - our three edge ratios (median, mean, p90 ÷ random-pair distance);
  - NP₁/NP₂ on sampled nodes, and on the quotient levels;
  - stress at optimal scale on sampled pairs;
  - ARI and silhouette against SNAP's ground-truth `com-lj` communities, not against our own hierarchy, which would be circular
- [ ] Time and anonymous peak per tool, labelled exploratory unless run in an official window

**Done when.** A table tool × graph × metric in FINDINGS, and a statement of where our layout is behind, level or ahead.

**Decision it feeds.** D17 is re-decided after this experiment, together with {{#m1-07-layout-area}} (`docs/research/README.md` B10). The result also feeds ADR 0017's "what would change our mind" metrics.

**Pointers.** `docs/research/sota-maps.md` § 3 D17 rows, § 4 challenges 3 and 6; `lab/e-map`, FINDINGS § E.

**Depends on** {{#m0-04-datasets}}
