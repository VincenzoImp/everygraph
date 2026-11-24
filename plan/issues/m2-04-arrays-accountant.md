---
title: Segmentable arrays, fallible allocation and the memory accountant
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P0, risk:medium, size:M, lab-evidence
---
**Why.** Two hard facts: on wasm32 no single allocation may exceed 2 GiB (the lab hit it at 275 M edges), and the only OOM in the whole lab came from untracked anonymous O(n) state (WCC under a 1 GB limit). The no-OOM promise (DESIGN D11) needs every large allocation to be planned and counted.

**Scope**
- [ ] `SegVec<T>`: a vector made of fixed-size segments (≤ 1 GiB), with slice-of-segments iteration that kernels can use without bounds-check overhead in the hot loop
- [ ] `MemoryAccountant`: a budget, reservations by named purpose, peak tracking, `try_reserve` that fails with a typed error instead of aborting
- [ ] All large buffers in the workspace allocated through it; a lint or test that fails on direct large `Vec::with_capacity` in kernel crates
- [ ] Accounting distinguishes anonymous memory (counted against the budget) from file-backed mappings (reclaimable) — RSS is misleading for mmap
- [ ] Benchmark: `SegVec` vs `Vec` in the PageRank inner loop (must be within 3%)

**Acceptance**
- A kernel asked to run with a budget below its O(n) state returns `InsufficientBudget { needed, available }` before allocating
- Under `MemoryMax` equal to the declared budget plus a fixed slack, no run in the test matrix is OOM-killed

**Feeds.** {{#m3-02-planner}}, {{#m3-10-no-oom-ci}}.
