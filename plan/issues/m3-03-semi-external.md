---
title: Semi-external streaming source with fixed memory
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P0, risk:medium, size:M, lab-evidence
---
**Why.** This is the regime behind "1.8 B edges in 2 GB". Lab: Friendster PageRank at 3.7 s/iteration with 1.35–1.47 GB RSS under 2 GB and 8 GB limits, reading 3.9 GB/s from NVMe with only ~5% of thread time in `read()` — the kernel is not disk-bound.

**Scope**
- [ ] Stream source over the compressed format: per-worker large aligned sequential reads, decode, hand chunks to the edge-map driver
- [ ] Buffered and direct modes chosen by the planner; read-ahead sizing from the remaining budget
- [ ] Pinned offsets/block directory in RAM (small), never adjacency in anonymous memory
- [ ] Multi-pass kernels reuse the stream without reopening
- [ ] Later: `io_uring` backend (raw cold scans through mmap reached only ~55% of disk bandwidth)

**Acceptance**
- Friendster PageRank under `MemoryMax=2G` within 1.35× of the in-RAM time, anonymous peak ≤ 1.6 GB
- Twitter-sym under an 8 GB cap: compressed stream ≥ 4× faster than raw mmap on repeated scans (lab: 5.2×)

**Pointers.** `lab/b1-core/crates/glab-core/src/extmem.rs`, FINDINGS § B1 and § C "cold".

**Depends on** {{#m3-01-edge-map}}
