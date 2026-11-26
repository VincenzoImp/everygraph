---
title: M4 epic: communities and hierarchy
milestone: M4 · Communities and hierarchy
labels: type:epic, area:community, P0
---
Tracking issue for milestone M4.

**Goal.** Production community detection — deterministic by default, with a fast mode — that works beyond RAM, meets quality gates on six graph families, and produces the hierarchy the map is built on.

**Lab baseline (idle machine, 16 threads).** Leiden async: lj 0.76 s (Q 0.7497), orkut 1.43 s (Q 0.6689). Leiden deterministic: 2.30 s (0.7480), 3.48 s (0.6760), identical hash at 1–32 threads. Three iterations: lj 0.7647–0.7651, orkut 0.6905–0.6922 (best idle reference: igraph Leiden, 2 iterations, 0.7695 and 0.6924, FINDINGS "Idle-machine re-runs"; lj's is the reference of DESIGN § 6). GVE-Leiden on the same idle machine: lj 1.02 s (Q 0.7223), orkut 1.45 s (Q 0.6636); NetworKit ParallelLeiden: 6.0 s (0.7688), 9.4 s (0.6720). Friendster in RAM: 154.5 s, Q 0.6033, 22.6 GiB anonymous (32 threads, shared machine). Idle, one async iteration at 16 threads (#118, one repetition each): 82.4 s raw / 82.0 s on the default storage, Q 0.6050 / 0.6008, anonymous peak 37.0 / 34.3 GiB with the graph in heap — compression does not bring it under 16 GiB.

**Exit criteria**
- [ ] {{#m4-01-leiden-production}}, {{#m4-02-weights-resolution}}
- [ ] Out-of-core: {{#m4-04-ooc-leiden}} — Friendster under 16 GiB with Q ≥ 0.60; the quality mode within 1% of the better of NetworKit PLM and GVE-Leiden run on this machine (DESIGN § 6, restated 2026-11-25; measured in {{#m4-01-leiden-production}})
- [ ] {{#m4-05-quality-gates}} — within 1% of the best reference on six families
- [ ] {{#m4-06-hierarchy-artifact}}, {{#m4-07-map-hierarchy}}, with {{#m4-10-per-region-jobs}} and {{#m4-11-labelled-binning}}
- [ ] {{#m4-08-community-tools}}; {{#m4-09-label-propagation}} (CDLP, needed by {{#m8-11-graphalytics-eligibility}})
- [ ] Milestone close (CLAUDE.md § 6, § 10): the dogfood pass, the cleanup and test audit, the official measurement window, and the report `docs/reports/M4.md`. Its issues are filed when the last P0 of the milestone starts

**M1 results it builds on (all closed):** {{#m1-02-ooc-aggregation}}, {{#m1-03-louvain-collapse}}, {{#m1-04-det-leiden-scaling}}, {{#m1-05-hierarchy-shaping}}.
