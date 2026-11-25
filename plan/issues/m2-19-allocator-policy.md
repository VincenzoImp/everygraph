---
title: Allocator policy: the budget must bound real memory, not only the ledger
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P0, risk:medium, size:M
---
**Why.** The memory accountant ({{#m2-04-arrays-accountant}}) keeps an exact ledger, but the process's anonymous memory can exceed it: the review of the identifier dictionary (PR #158) measured anonymous peaks of 1.2–2.3× the budget during builds whose ledger stayed within it (128 MiB budget, URL-like keys: 241 MiB, OOM-killed under `MemoryMax=200M`), caused by glibc arena retention and fragmentation; `MALLOC_ARENA_MAX=1 MALLOC_TRIM_THRESHOLD_=0` brought it to 122 MiB. The product's second promise is about real memory (`design/DECISIONS.md`, "The memory promise covers the process").

**Scope**
- [ ] Compare, with interleaved A/B runs on the dictionary build, the text reader and a kernel: glibc with arena and trim settings applied at start-up, `mimalloc`, `jemalloc` (purge settings) — anonymous peak against the ledger, time, licence (`cargo deny`), build on every tier-1 platform; wasm32 keeps its own allocator
- [ ] Choose for the command line and the Python module; document what an embedding Rust program must do to get the guarantee
- [ ] A headroom rule in the planner for what no allocator can avoid (thread stacks, fragmentation), measured
- [ ] A test in {{#m3-10-no-oom-ci}}: anonymous peak ≤ budget + headroom under `MemoryMax` for the dictionary, ingest and two kernels

**Acceptance**
- On the review's cases the anonymous peak stays within budget + the stated headroom, and no case is OOM-killed under `MemoryMax` = budget + headroom
