---
title: Past the memory-bandwidth ceiling: blocked PageRank and locality
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:engine, P1, risk:medium, size:M, lab-evidence
---
**Question.** On Friendster, PageRank costs 3.4 s/iteration at 32 threads — about 11× more per edge than on orkut, whose state fits in L3 — and it looked flat from 8 threads up. That flatness was only observed while the machine was loaded (about 4.8, 5.5, 4.1 s at 8, 16, 32 threads — read off the screen in the first lab, not kept in a result file, so an observation, not a measurement); no idle scaling curve exists for Friendster. Is this the memory-bandwidth limit of a dual-channel desktop, and how much can cache-aware techniques recover?

**Method**
- [ ] First, an idle-machine scaling curve at 1, 2, 4, 8, 16, 32 threads
- [ ] Confirm with hardware counters (`perf stat`: LLC misses, memory bandwidth) at 4, 8, 16 threads
- [ ] Zero-code step before any prototype: DRAM bytes per adjacency entry of the current pull kernel on Friendster at 16 threads, from the AMD UMC counters. The review's estimate, to be checked: about 53 B/entry today against about 15–20 B/entry binned (`docs/research/sota-kernels.md` challenge 5, arithmetic)
- [ ] Propagation-blocked PageRank, deterministic by construction (sharpened 2026-11-25, `docs/research/README.md` B13):
  - a push scan of the compressed adjacency in ascending source order into bins of 64 K destinations (f64 value + u16 local id), accumulated bin by bin;
  - bins are per **fixed source chunk**, not per thread, and are read in chunk order. Each destination then receives its contributions in ascending source order, whatever the bin width and thread count;
  - it can be made bit-identical to the current pull if the accumulator reproduces the pull's 4-lane association.

  Transient bins take 10 B/entry (Friendster about 36 GB unchunked), so source chunks are sized by the budget; wasm has no streaming stores. Published: 1.75× on the friends graph (GAP pull 3.72 → 2.12 s/iteration) and 1.48× on Twitter (Beamer et al., IPDPS'17, 2 × 8-core Ivy Bridge, 16 threads, their machine). Lab A/B against pull, interleaved: Friendster, twitter-sym, and orkut as a control, 16 threads, s/iteration and hash identity
- [ ] Node orderings on Friendster and twitter: original, degree-sorted, hub-clustered, BFS, Leiden-hierarchy order ({{#m1-09-hierarchy-order}})
- [ ] f32 vs f64 state; compressed vs raw source (#118: at 16 threads compressed scans beat raw on all four graphs (14.9–27.5 vs 12.4–13.7 G entries/s); orkut PageRank IPC 0.49 raw vs 1.97 compressed; Friendster PageRank 3.19 vs 3.36 s/iteration)
- [ ] Same questions for the Leiden local-moving phase

**Success.** ≥ 1.5× on Friendster PageRank at 16 threads, or a documented explanation of why not.

**Decision it feeds.** D13 defaults and whether the edge-map framework ({{#m3-01-edge-map}}) needs a blocked variant from the start.
