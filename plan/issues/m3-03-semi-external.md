---
title: Semi-external streaming source with fixed memory
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P0, risk:medium, size:M, lab-evidence
---
**Why.** This is the regime behind "1.8 B edges in 2 GB". Lab: Friendster PageRank at 3.6–3.7 s/iteration with 1.35–1.47 GB RSS under 2 GB and 8 GB limits, reading 3.9 GB/s from NVMe with only ~5% of thread time in `read()` — the kernel is not disk-bound. #118: through plain mmap of the compressed store, Friendster PageRank under 2 GB is 4.31 s/iteration vs 3.19 in RAM — 1.35×, no margin; the raw-format streaming reader achieved 1.08×. A streaming reader for the compressed format is the work of this issue.

**Scope**
- [x] Stream source over the compressed format: per-worker large aligned sequential reads, decode, hand chunks to the edge-map driver
- [x] Buffered and direct modes chosen by the planner; read-ahead sizing from the remaining budget
- [x] Pinned offsets/block directory in RAM (small), never adjacency in anonymous memory
- [x] Multi-pass kernels reuse the stream without reopening
- [x] **Alternating scan direction** for multi-pass kernels whose result does not depend on the visiting order (Grafu's "elevator" order, ASPLOS 2024; added 2026-11-25, `docs/research/README.md` B14). Forward, then backward: the tail of the previous pass is still in the page cache, where a forward-only scan larger than the cache gets nothing from LRU. Applies to Jacobi PageRank (fixed reduction blocks, still combined in a fixed order), WCC rounds, SemiCore* and HyperBall. Cheap test: Friendster PageRank on the compressed store (about 10 GB) under `MemoryMax=8G`, forward-only against alternating, 10 iterations; `read_bytes` per iteration and an identical output hash. The same on orkut with a budget at half its compressed size
- [ ] **Decode only the active lists** for monotone fixpoint kernels (min-label WCC, BFS depth, SSSP, k-core), which have a unique answer, so the order of work cannot change it (`docs/research/memory-bounded-algorithms.md` addendum, "One correction to the survey above", and § F 5). Two techniques: on-demand reading of active nodes' lists through the Elias–Fano offsets (Seraph, FAST 2024), and bound-based pruning (Gem, SIGMOD 2026). Cheap test: Friendster WCC out of core with per-round active fractions logged; bytes read when decoding only active lists, against the full scan's 3.6 s; labels asserted identical. Not for order-sensitive kernels (PageRank sums, Leiden, LPA). On-demand reads are built (`CompressedStream`, #254); bound-based pruning is not
- [ ] Core numbers within a fixed memory: SemiCore* (Wen et al., ICDE 2016; exact, O(n) memory, re-reads only the nodes whose estimate may change) over this source, identical to the in-RAM peel (moved from {{#m3-07-kcore-degrees}} when it closed; without it the planner refuses k-core on a graph that does not fit)
- [ ] Later: `io_uring` backend (raw cold scans through mmap reached only ~55% of disk bandwidth)
- [ ] The in-lists and the undirected projection of a directed graph streamed too: a stream of each, of one type for the kernels that take both (`Plan::with_pair`), and a merge that reads its two inputs by positional reads. Until then `PageRank`, strong components, core numbers, degrees and searches on a directed graph whose file holds its in-lists are held or mapped (PR #PRNUM)

**Acceptance**
- Friendster PageRank under `MemoryMax=2G` on the compressed format within 1.35× of the in-RAM time on the same format (3.19 s/iteration, f32, 16 threads), anonymous peak ≤ 1.6 GB
- Twitter-sym under an 8 GB cap: repeated scans of the compressed form (4.8 GB, stays resident) ≥ 4× faster than of raw arrays (10 GB, re-read from disk every pass); lab: 5.2×. #118 measured PageRank 1.9× and WCC 1.3× at 8 GB through plain mmap; the ≥ 4× target is for repeated scans and stays
- Stated plainly in the docs: out of core, each pass costs about one sequential read of the graph, so kernels that are cheap in RAM slow down by the ratio of read time to compute time (WCC on Friendster: 0.34 s in RAM, 3.6 s streamed)

**Pointers.** `lab/b1-core/crates/glab-core/src/extmem.rs`, FINDINGS § B1 and § C "cold".

**Depends on** {{#m3-01-edge-map}}
