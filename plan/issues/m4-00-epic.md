---
title: M4 epic: communities and hierarchy
milestone: M4 · Communities and hierarchy
labels: type:epic, area:community, P0
---
Tracking issue for milestone M4.

**Goal.** Production community detection — deterministic by default, with a fast mode — that works beyond RAM, meets quality gates on six graph families, and produces the hierarchy the map is built on.

**Lab baseline (idle machine, 16 threads).** Leiden async: lj 0.76 s (Q 0.7497), orkut 1.43 s (Q 0.6689). Leiden deterministic: 2.30 s (0.7480), 3.48 s (0.6760), identical hash at 1–32 threads. Three iterations: lj 0.7647–0.7651, orkut 0.6905–0.6922 (best reference: 0.7704, 0.6911). GVE binaries on the same machine: 1.7–2.5 s at Q 0.65–0.72. Friendster in RAM: 154.5 s, Q 0.6033, 23.6 GB anonymous.

**Exit criteria**
- [ ] {{#m4-01-leiden-production}}, {{#m4-02-weights-resolution}}, {{#m4-03-parallel-aggregation}}
- [ ] Out-of-core: {{#m4-04-ooc-leiden}} — Friendster under 16 GiB with Q ≥ 0.60
- [ ] {{#m4-05-quality-gates}} — within 1% of the best reference on six families
- [ ] {{#m4-06-hierarchy-artifact}}, {{#m4-07-map-hierarchy}}
- [ ] {{#m4-08-community-tools}}; optional {{#m4-09-label-propagation}}

**Blocked by M1 results:** {{#m1-02-ooc-aggregation}}, {{#m1-03-louvain-collapse}}, {{#m1-04-det-leiden-scaling}}, {{#m1-05-hierarchy-shaping}}.
