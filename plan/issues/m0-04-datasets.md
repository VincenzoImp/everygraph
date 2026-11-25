---
title: Dataset registry and fetch scripts
milestone: M0 · Foundations and decisions
labels: type:infra, area:bench, P0, risk:low, size:S
---
**Why.** Benchmarks and quality gates need the same inputs everywhere, with checksums, and the first lab covered only social graphs.

**Scope**
- [ ] `datasets.toml`: name, URL(s), size, SHA-256, licence/terms, node/edge counts, directed?, weighted?, family
- [ ] Fetcher with parallel ranged download (SNAP serves ~0.7 MB/s per connection; 8 ranges gave ~17 MB/s) and checksum verification
- [ ] Tier 1 (CI, < 1 GB, in practice under 100 MB so an uncached CI run stays cheap): small social graphs, a road graph, a web graph, a temporal edge list, and generated graphs with known answers, among them a bipartite one and one with positive weights. Only data whose terms allow use by anyone: no dataset restricted to research or non-commercial use (decided 2026-11-25 after the review of PR #129; MovieLens 100K was dropped for this reason)
- [ ] Tier 2 (workstation): com-LiveJournal (moved from tier 1 on 2026-11-25: 124 MB, more than twice the rest of tier 1, and nothing in CI needs a graph that size), com-Orkut, twitter-2010 (LAW BVGraph), uk-2005 (web), com-Friendster
- [ ] Tier 3 (server, optional): a ≥ 10 B-edge web graph from LAW
- [ ] No dataset is redistributed in the repository; terms are recorded per dataset

**Acceptance**
- `everygraph-bench fetch tier1` produces verified files from a clean machine
- Each family in {{#m1-10-graph-families}} has at least one registered dataset

**Depends on** {{#m0-01-workspace}}
