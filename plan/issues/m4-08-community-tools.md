---
title: Community comparison and inspection tools
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P2, risk:low, size:S
---
**Why.** Users need to judge a partition, compare runs and parameters, and export results — beyond a single modularity number.

**Scope**
- [ ] Partition comparison: NMI, ARI, variation of information; contingency table as Arrow
- [ ] Per-community metrics: size, density, conductance, internal/external degree, enrichment vs the configuration-model expectation
- [ ] Stability report across seeds and resolutions
- [ ] Significance caveats in the docs: modularity's resolution limit, communities found in random graphs

**Acceptance**
- Metrics equal scikit-learn / igraph implementations on oracle partitions

**Depends on** {{#m4-01-leiden-production}}
