---
title: Dataset registry and fetch scripts
milestone: M0 · Foundations and decisions
labels: type:infra, area:bench, P0, risk:low, size:S
---
**Why.** Benchmarks and quality gates need the same inputs everywhere, with checksums, and the first lab covered only social graphs.

**Scope**
- [ ] `datasets.toml`: name, URL(s), size, SHA-256, licence/terms, node/edge counts, directed?, weighted?, family
- [ ] Fetcher with parallel ranged download (SNAP serves ~0.7 MB/s per connection; 8 ranges gave ~17 MB/s) and checksum verification
- [ ] Tier 1 (CI, < 1 GB): com-LiveJournal, a road graph, a small bipartite graph, a weighted graph, a temporal edge list
- [ ] Tier 2 (workstation): com-Orkut, twitter-2010 (LAW BVGraph), uk-2005 (web), com-Friendster
- [ ] Tier 3 (server, optional): a ≥ 10 B-edge web graph from LAW
- [ ] No dataset is redistributed in the repository; terms are recorded per dataset

**Acceptance**
- `everygraph-bench fetch tier1` produces verified files from a clean machine
- Each family in {{#m1-10-graph-families}} has at least one registered dataset

**Depends on** {{#m0-01-workspace}}
