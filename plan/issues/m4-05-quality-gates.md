---
title: Quality gates against reference implementations
milestone: M4 · Communities and hierarchy
labels: type:infra, area:community, P0, risk:medium, size:M, lab-evidence
---
**Why.** Speed without quality is a trap. Quality must be a CI gate, measured with one common routine.

**Scope**
- [ ] One evaluation routine (modularity at γ, number of communities, disconnected communities, size distribution) applied to our output and to every reference partition — as `b2 eval` did in the lab
- [ ] References with pinned versions: NetworKit ParallelLeiden and PLM, igraph multilevel and Leiden, leidenalg, GVE
- [ ] Graph families from {{#m1-10-graph-families}}: social, web, road, bipartite, weighted, directed
- [ ] The default number of Leiden iterations confirmed or changed: quality and time at 1, 2, 3 iterations and at convergence on these families, recorded in FINDINGS; a change of default goes through an ADR (`design/DECISIONS.md`, 2026-11-25; moved here when {{#m4-01-leiden-production}} closed)
- [ ] Planted-partition benchmarks (LFR, degree-corrected SBM) with NMI/ARI against ground truth, across mixing levels — modularity alone is not evidence of correct communities
- [ ] Gates: quality mode within 1% of the best reference modularity per graph; default mode within 3%; NMI on LFR within 0.02 of leidenalg
- [ ] The reference of each gate is named per graph, with its tool, version, iterations, repetitions and machine; it is run on this machine with the same graph and preprocessing (DESIGN § 6, restated 2026-11-25). lj: igraph Leiden 0.7695, one run of two iterations (FINDINGS "Idle-machine re-runs"). Traag et al.'s 0.7739 (best of 10 replications × 10 iterations, their machine) is shown beside it as the published best, not used as the gate: against it our three-iteration 0.7647 is 1.2% below, which is a fair finding, not a regression (`docs/research/sota-communities.md` challenge 9). Friendster: the better of NetworKit PLM and GVE-Leiden on this machine
- [ ] Beyond modularity (`docs/research/sota-communities.md` § 3, m4-05 row): seed-to-seed ARI as the baseline for every stability number; conductance and triad-participation ratio of found communities; precision and recall against SNAP's top-5,000 ground-truth communities on lj, orkut and Friendster (Yang–Leskovec: modularity ranks ground-truth communities "in nearly reverse order")
- [ ] Track quality drift with thread count in `fast` mode (seen on orkut: 0.6836 at 1 thread → 0.6743 at 32)

**Acceptance**
- Nightly job produces the quality table; a regression beyond the gates fails the build

**Depends on** {{#m4-01-leiden-production}}, {{#m0-03-bench-harness}}, {{#m1-10-graph-families}}
