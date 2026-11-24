---
title: Performance tracking against reference tools
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P1, risk:low, size:M, lab-evidence
---
**Why.** "Faster than X" claims rot. They need to be regenerated continuously and published with the method.

**Scope**
- [ ] Benchmark specs per kernel on lj, orkut, twitter, Friendster at 1 and 16 threads, three regimes
- [ ] Reference runs with pinned versions: NetworKit, python-igraph, scipy, GVE-Louvain/Leiden, DuckDB (ingest), plus the query engines from the F2 comparison
- [ ] Fairness notes per comparison: same input, same parameters, load time reported separately, what each tool includes
- [ ] Regression gate: a PR that slows a tracked benchmark by > 7% on the self-hosted runner needs an explicit waiver
- [ ] Published dashboard page generated from result files ({{#m8-03-public-benchmarks}})

**Acceptance**
- Weekly run produces the comparison tables automatically
- Every claim in the README links to a spec and a result file

**Depends on** {{#m0-03-bench-harness}}
