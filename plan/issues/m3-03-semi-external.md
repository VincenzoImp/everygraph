---
title: Semi-external streaming source with fixed memory
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P0, risk:medium, size:M, lab-evidence
---
**Why.** This is the regime behind "1.8 B edges in 2 GB". Lab: Friendster PageRank at 3.7 s/iteration with 1.35–1.47 GB RSS under 2 GB and 8 GB limits, reading 3.9 GB/s from NVMe with only ~5% of thread time in `read()` — the kernel is not disk-bound. #118: through plain mmap of the compressed store, Friendster PageRank under 2 GB is 4.31 s/iteration vs 3.19 in RAM — 1.35×, no margin; the raw-format streaming reader achieved 1.08×. A streaming reader for the compressed format is the work of this issue.

**Scope**
- [ ] Stream source over the compressed format: per-worker large aligned sequential reads, decode, hand chunks to the edge-map driver
- [ ] Buffered and direct modes chosen by the planner; read-ahead sizing from the remaining budget
- [ ] Pinned offsets/block directory in RAM (small), never adjacency in anonymous memory
- [ ] Multi-pass kernels reuse the stream without reopening
- [ ] Later: `io_uring` backend (raw cold scans through mmap reached only ~55% of disk bandwidth)

**Acceptance**
- Friendster PageRank under `MemoryMax=2G` on the compressed format within 1.35× of the in-RAM time on the same format (3.19 s/iteration, f32, 16 threads), anonymous peak ≤ 1.6 GB
- Twitter-sym under an 8 GB cap: repeated scans of the compressed form (4.8 GB, stays resident) ≥ 4× faster than of raw arrays (10 GB, re-read from disk every pass); lab: 5.2×. #118 measured PageRank 1.9× and WCC 1.3× at 8 GB through plain mmap; the ≥ 4× target is for repeated scans and stays
- Stated plainly in the docs: out of core, each pass costs about one sequential read of the graph, so kernels that are cheap in RAM slow down by the ratio of read time to compute time (WCC on Friendster: 0.34 s in RAM, 3.6 s streamed)

**Pointers.** `lab/b1-core/crates/glab-core/src/extmem.rs`, FINDINGS § B1 and § C "cold".

**Depends on** {{#m3-01-edge-map}}
