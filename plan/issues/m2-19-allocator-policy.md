---
title: Allocator policy: the budget must bound real memory, not only the ledger
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P0, risk:medium, size:M
---
**Why.** The memory accountant ({{#m2-04-arrays-accountant}}) keeps an exact ledger, but the process's anonymous memory can exceed it: the review of the identifier dictionary (PR #158) measured anonymous peaks of 1.2–2.3× the budget during builds whose ledger stayed within it (128 MiB budget, URL-like keys: 241 MiB, OOM-killed under `MemoryMax=200M`), caused by glibc arena retention and fragmentation; `MALLOC_ARENA_MAX=1 MALLOC_TRIM_THRESHOLD_=0` brought it to 122 MiB. The product's second promise is about real memory (`design/DECISIONS.md`, "The memory promise covers the process").

**Scope** (amended 2026-11-25, `docs/research/sota-io-memory.md` § 3 "The memory promise covers the process", § 4 C2 and C3; `docs/research/README.md` B8, B9)
- [ ] **Large buffers never go through `malloc`.** `SegVec` segments, `ScratchPool` slots and bucket buffers above about 1 MiB come from a page backend injected through `Memory`: anonymous mmap on native targets (2 MiB aligned, THP allowed, `munmap` on drop) and `Vec` on wasm32. The backend is a trait implemented in `everygraph-io`, so the core keeps no OS calls. About 40 lines of `unsafe`: tier-2 review. Velox, Umbra and vmcache do the same, which makes the retention of large buffers zero by construction. It also works inside Python, whose allocator an extension cannot replace
- [ ] **glibc pinned in the CLI at start-up:** `mallopt(M_ARENA_MAX, 2)`, a fixed `M_MMAP_THRESHOLD` and `M_TRIM_THRESHOLD`, and `malloc_trim(0)` between stages, through the `libc` crate. Setting any of them turns off glibc's dynamic mmap threshold, which otherwise rises to 32 MiB after a free, so later allocations below 32 MiB come from arenas and are kept (mallopt(3), glibc `malloc.c`). That matches PR #158's 1.2–2.3× retention (the review's inference)
- [ ] mimalloc v3 (MIT; `purge_delay=0`, `purge_decommits=1`) as an A/B candidate for the CLI only; the `mimalloc` and `tikv-jemallocator` crates compile C, so they can never be the core's allocator
- [ ] **Verify before relying on it:** the review states that Rust's `Allocator` trait is stable since 1.100.0 (installed rustc 1.100.0). Check the release notes and the standard library documentation. The workspace MSRV is 1.87, so using it to make `SegVec` segments `Vec<T, A>` would need an MSRV decision. Until both are settled, the page backend goes through `Memory`, not `Allocator`
- [ ] **A cgroup monitor that reconciles the ledger while running** (C3; Linux only, a no-op elsewhere). Every 100 ms it reads `memory.current`, `memory.stat` (`anon`, `pagetables`, `kernel`, `shmem`, `workingset_refault_file`, `pgmajfault`) and `memory.pressure`. On anonymous drift beyond the headroom it trims or purges. It may stop or refuse a run, never switch precision or change a result. ClickHouse resets its tracker to the cgroup or jemalloc value every 50–100 ms because drift otherwise makes "every allocation fail"
- [ ] The headroom rule gains kernel terms the cgroup charges and the ledger does not see: page tables of file maps (about 2 MiB per GiB touched with 4 KiB pages, freed only by `munmap`) and kernel stacks (16 KiB per thread)
- [ ] Compare, with interleaved A/B runs on the dictionary build, the text reader and a kernel: glibc with arena and trim settings applied at start-up, `mimalloc`, `jemalloc` (purge settings) — anonymous peak against the ledger, time, licence (`cargo deny`), build on every tier-1 platform; wasm32 keeps its own allocator
- [ ] Choose for the command line and the Python module; document what an embedding Rust program must do to get the guarantee
- [ ] A headroom rule in the planner for what no allocator can avoid (thread stacks, fragmentation), measured
- [ ] A test in {{#m3-10-no-oom-ci}}: anonymous peak ≤ budget + headroom under `MemoryMax` for the dictionary, ingest and two kernels

- [ ] The A/B of C2: PR #158's dictionary build at a 128 MiB budget with URL keys, four variants interleaved: glibc default; `mallopt` pins; pins + page backend; mimalloc. Metric: anonymous peak ÷ ledger peak, and time
- [ ] The monitor experiment of C3: replay the tracer's Leiden-16G case (lab b2, `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 9, killed at 460.5 s) with the monitor logging only. Measure the time to detection against the time to kill

**Acceptance**
- On the review's cases the anonymous peak stays within budget + the stated headroom, and no case is OOM-killed under `MemoryMax` = budget + headroom
