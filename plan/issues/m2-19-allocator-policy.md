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
- [ ] **Method: test the pin by its effect, not by `mallopt`'s return code** (review of PR #203). A unit test in `src/sys.rs`, run in its own process because `mallinfo2` is process-wide: free an 8 MiB block, allocate 1 MiB, and check that `mallinfo2().hblkhd` grew by at least 1 MiB. On glibc 2.41 here it was false without the pin and true with it. The test must fail when the pin is removed
- [ ] **Method: a fixed threshold of 2–4 MiB as an A/B candidate** against 128 KiB. Any fixed value turns off the dynamic growth. At 128 KiB every block of 128 KiB or more is a fresh map: it cannot get THP, and its pages fault in again. That is the probable cause of the +0.3 s on in-memory phases below (an inference, not measured)
- [ ] `mallopt` is `MT-Unsafe init const:mallopt` (glibc manual): call it before any thread starts, behind an interface that makes this the caller's obligation. Say that it affects glibc's `malloc` only, not a program's own `#[global_allocator]`
- [ ] Replace the stopgap of PR #203 (`GLIBC_TUNABLES` in the `env` of the `build-500m` case of `bench/specs/id-dictionary.toml`), then rerun that case without it
- [ ] Run the scale of the retention at 4, 8 and 16 threads. "Threads × the 32 MiB dynamic maximum" is an inference from 16-thread runs only

**Evidence so far** (PR #203; exploratory runs, not for documents). Setup:
- `id_dictionary --keys ethbin --scratch dir:… --layout split`, 16 threads.
- Each run in its own `systemd-run --scope` with `MemoryMax=5G`, no swap.
- Budget 4 GiB × n / 500 M. Load average 17–37, so the times are not comparable between rows.
- b = `--build-only yes`.
- Variants: "pin env" is `MALLOC_ARENA_MAX=1 MALLOC_TRIM_THRESHOLD_=0`; "in-process" is `mallopt` of the mmap and trim thresholds at 128 KiB.
- Binaries: main 32d965d; pre f477378, before PR #158's second-review fixes. "scratch fix" removes the 1 MiB `BufWriter` that the benchmark's scratch put behind each of about 190 open writers.

| n | binary | glibc | result | anon peak | ÷ ledger peak |
|---|---|---|---|---|---|
| 50 M b | main | default | ok | 909, 974 MiB | 2.22, 2.38 |
| 50 M b | main | `MALLOC_MMAP_THRESHOLD_=131072` only | ok | 367 MiB | 0.89 |
| 50 M b | main | `MALLOC_ARENA_MAX=2` only | ok | 570 MiB | 1.39 |
| 50 M b | main | `ARENA_MAX=2`, mmap and trim 128 KiB | ok | 456 MiB | 1.11 |
| 50 M b | main | pin env | ok | 404 MiB | 0.98 |
| 50 M b | scratch fix | default | ok | 749 MiB | 1.83 |
| 50 M b | scratch fix | in-process | ok | 405 MiB | 0.99 |
| 100 M | main / pre | default | ok | 1366 / 1395 MiB | 1.67 / 1.70 |
| 200 M | main / pre | default | ok | 2177 / 2266 MiB | 1.33 / 1.38 |
| 200 M b | scratch fix | default / in-process | ok | 2136 / 1554 MiB | 1.30 / 0.95 |
| 500 M | main | default | ok, 262 MiB from the limit | 4858 MiB | 1.19 |
| 500 M | pre | default | OOM-killed at 94 s | 4991 MiB | 1.22 |
| 500 M | main / pre | pin env | ok | 3619 / 3659 MiB | 0.88 / 0.89 |
| 500 M | scratch fix | default | ok | 4852 MiB | 1.18 |
| 500 M | scratch fix | in-process (2 runs) | ok | 3755, 3590 MiB | 0.92, 0.88 |

Retention: anonymous peak − ledger peak, main, default glibc.

| n | 50 M | 100 M | 200 M | 500 M |
|---|---|---|---|---|
| MiB | 499, 544, 564 | 546 | 537, 656 | 762 |

It is flat in n, so it is not O(n) state. In the 500 M runs the anonymous memory peaked in phase C (2.4 ↔ 4.8 GiB against a ledger of 3.84–3.87 GiB). That this is where the official run died (`bench/results/id-dictionary/20261125T220117Z-c855fdd77cf9.jsonl`, killed at 47 s, phase not recorded) is an inference from the exploratory runs.

A counting allocator showed that the library's own heap stays within its ledger: 0 MiB above it at 50 M with the scratch fix, 4 MiB at 500 M. With the 1 MiB buffers, the heap went up to 114 MiB above the ledger at 50 M.

Cost of the in-process pin: interleaved A/B, 4 rounds, 15 M hex keys, `--scratch memory`, load about 28. `phases_s` went from 1.39–1.47 s to 1.65–1.75 s, `build_s` from a mean of 2.89 s to 2.97 s, and cgroup `memory.peak` from 3.2 GB to 2.6 GB.

**Acceptance**
- On the review's cases the anonymous peak stays within budget + the stated headroom, and no case is OOM-killed under `MemoryMax` = budget + headroom
