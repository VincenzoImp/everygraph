---
title: In-adjacency for directed graphs as a derived artifact
milestone: M2 · Format and ingest
labels: type:feature, area:format, P1, risk:low, size:S, lab-evidence
---
**Why.** In-neighbors, SCC, pull-based PageRank on directed graphs and directed bidirectional BFS need the transpose. Lab: parallel transpose of twitter-2010 (1.47 B arcs) in 12.3 s at 32 threads; raw cost +4.23 B/arc.

**Scope**
- [ ] Parallel transpose (count, prefix, scatter, per-list sort) writing the standard compressed format
- [ ] Out-of-core variant through the bucket machinery of {{#m2-08-csr-build}}
- [ ] Stored as a derived section, built at ingest for directed inputs by default and skippable
- [ ] Edge-id column so properties resolve from either direction

**Acceptance**
- twitter-2010 transpose ≤ 15 s at 16 threads in RAM
- For every arc (u,v) in the out-adjacency, v's in-list contains u (verified by {{#m2-14-verify}})
