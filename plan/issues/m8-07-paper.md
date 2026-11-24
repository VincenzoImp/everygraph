---
title: System paper
milestone: M8 · CLI, docs and v0.1 launch
labels: type:docs, area:launch, P2, risk:medium, size:L
---
**Why.** A peer-reviewed description gives the project a citable reference and forces the evaluation to be rigorous. Candidate venues: VLDB / SIGMOD (systems or demo track), IEEE VIS (the map), or a software journal.

**Possible contributions (to be confirmed by M1–M5 results)**
- A scale-adaptive single-machine graph engine with a hard memory budget, chosen by O(n)-state size
- Thread-count-independent deterministic Leiden and its measured cost
- A hierarchical data-tile map of billion-edge graphs with exact aggregates, in a static range-request file
- Negative results worth publishing: FST vs MPHF for id dictionaries, masks vs materialized views, compressed vs raw adjacency at high thread counts

**Scope**
- [ ] Related-work pass (ACGraph, GBBS, webgraph-rs, GVE-Leiden, LD-Leiden, GraphMaps/MSAGLJS, Uncharted Salt, Graphia, cosmos.gl)
- [ ] Evaluation plan reusing the public benchmark kit ({{#m8-03-public-benchmarks}})
- [ ] Artifact for reproducibility badges

**Acceptance**
- Submitted draft with a reproducible artifact
