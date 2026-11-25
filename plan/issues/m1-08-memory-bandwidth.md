---
title: Past the memory-bandwidth ceiling: blocked PageRank and locality
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:engine, P1, risk:medium, size:M, lab-evidence
---
**Question.** On Friendster, PageRank costs 3.4 s/iteration at 32 threads — about 11× more per edge than on orkut, whose state fits in L3 — and it looked flat from 8 threads up. That flatness was only observed while the machine was loaded (about 4.8, 5.5, 4.1 s at 8, 16, 32 threads — read off the screen in the first lab, not kept in a result file, so an observation, not a measurement); no idle scaling curve exists for Friendster. Is this the memory-bandwidth limit of a dual-channel desktop, and how much can cache-aware techniques recover?

**Method**
- [ ] First, an idle-machine scaling curve at 1, 2, 4, 8, 16, 32 threads
- [ ] Confirm with hardware counters (`perf stat`: LLC misses, memory bandwidth) at 4, 8, 16 threads
- [ ] Propagation blocking / cache-blocked pull PageRank (bin contributions by destination range)
- [ ] Node orderings on Friendster and twitter: original, degree-sorted, hub-clustered, BFS, Leiden-hierarchy order ({{#m1-09-hierarchy-order}})
- [ ] f32 vs f64 state; compressed vs raw source (compressed scans beat raw at 32 threads on twitter)
- [ ] Same questions for the Leiden local-moving phase

**Success.** ≥ 1.5× on Friendster PageRank at 16 threads, or a documented explanation of why not.

**Decision it feeds.** D13 defaults and whether the edge-map framework ({{#m3-01-edge-map}}) needs a blocked variant from the start.
