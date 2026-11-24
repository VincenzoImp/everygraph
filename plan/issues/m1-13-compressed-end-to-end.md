---
title: The default storage end to end: kernels, Leiden and queries on compressed adjacency
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:format, P0, risk:high, size:M, lab-evidence
---
**Question.** DESIGN D4 makes compressed adjacency the default storage, but the pieces were measured separately: every kernel, community and point-query number in the first lab was taken on *uncompressed* arrays, while the compressed format was measured only for scans and one PageRank iteration (2× slower than raw on orkut, equal on twitter-sym). What do the headline numbers become on the default format, and when must the planner decode to raw?

**Why it matters.** The public claims (PageRank 0.43 s on orkut, Leiden 0.76 s on lj, neighbours in 0.2–0.6 µs, shortest path in 0.3–1.5 ms) are all raw-array numbers. If the default format makes them 2–3× slower, either the default or the claims must change.

**Method**
- [ ] Plug the Stream VByte block decoder from `lab/c-compression` into the `lab/b1-core` kernels, `lab/b2-leiden` local moving and the `lab/f1-query` point queries
- [ ] Measure on lj, orkut, twitter-sym and Friendster, idle machine, at 1 and 16 threads: PageRank, WCC, BFS, k-core, triangles (intersection needs random access to long lists), Leiden (async and deterministic), neighbours, 2-hop, bidirectional BFS
- [ ] Three configurations: raw arrays; compressed with on-the-fly decode; compressed on disk with a raw copy decoded into RAM at open (report decode time and memory)
- [ ] Derive the planner rule: for which kernels and graph-size-to-RAM ratios is decoding to raw worth it

**Done when.** A table kernel × graph × configuration, a recommended default per kernel class, and DESIGN § 2 and § 6 numbers restated for the default configuration.

**Feeds.** {{#m2-03-adjacency-codec}}, {{#m3-02-planner}}, {{#m7-01-point-queries}}, and the wording of every public performance claim ({{#m8-03-public-benchmarks}}).
