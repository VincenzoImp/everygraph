---
title: Sampled centralities: betweenness, closeness, harmonic
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P2, risk:medium, size:M
---
**Why.** Analysts ask for betweenness constantly, and exact algorithms are infeasible beyond ~10^6 edges. Not covered by the lab.

**Scope**
- [ ] Approximate betweenness by source sampling with stated error bounds (Riondato–Kornaropoulos / KADABRA-style adaptive sampling)
- [ ] Harmonic and closeness centrality via multi-source BFS batches ({{#m3-06-traversal}}) or HyperLogLog-based neighborhood functions
- [ ] Degree and eigenvector centrality for completeness
- [ ] Every approximate result states its sample size and error bound in the output metadata

**Acceptance**
- On graphs where exact betweenness is feasible, the top-100 ranking overlaps ≥ 90% with exact at the default budget
- orkut approximate betweenness in < 60 s

**Depends on** {{#m3-06-traversal}}
