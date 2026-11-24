---
title: Past the memory-bandwidth ceiling: blocked PageRank and locality
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:engine, P1, risk:medium, size:M, lab-evidence
---
**Question.** Friendster PageRank is flat from 8 threads up (≈ 3.4–3.8 s/iteration) while orkut, whose state fits in L3, is 11× cheaper per edge. Is this the memory-bandwidth limit of a dual-channel desktop, and how much can cache-aware techniques recover?

**Method**
- [ ] Confirm with hardware counters (`perf stat`: LLC misses, memory bandwidth) at 4, 8, 16 threads
- [ ] Propagation blocking / cache-blocked pull PageRank (bin contributions by destination range)
- [ ] Node orderings on Friendster and twitter: original, degree-sorted, hub-clustered, BFS, Leiden-hierarchy order ({{#m1-09-hierarchy-order}})
- [ ] f32 vs f64 state; compressed vs raw source (compressed scans beat raw at 32 threads on twitter)
- [ ] Same questions for the Leiden local-moving phase

**Success.** ≥ 1.5× on Friendster PageRank at 16 threads, or a documented explanation of why not.

**Decision it feeds.** D13 defaults and whether the edge-map framework ({{#m3-01-edge-map}}) needs a blocked variant from the start.
