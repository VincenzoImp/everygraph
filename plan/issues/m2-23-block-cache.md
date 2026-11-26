---
title: Budgeted block cache over ByteSource for random access out of core
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P1, risk:medium, size:L
---
**Why.** Under a budget smaller than its working set, a random-access stage through a memory map re-reads the store many times: 733 GB for the tracer's layout on a 15.5 GB graph under 16 GiB (FINDINGS § M1-01). The page cache also makes resident memory differ from the ledger. An explicit cache over the portable `ByteSource` trait (D2) keeps the budget exact, works over `pread`, OPFS and HTTP ranges alike, and caches bytes, not results, so determinism is untouched (`docs/research/sota-io-memory.md` C1). {{#m1-15-mmap-hints-block-cache}} measured a prototype (ADR 0029, proposed): it reads close to the needed bytes (62.5 GB at 8G and 87.3 GB at 4G for Friendster level-0 local moving and refinement, against 375 GB and 799–933 GB through plain mmap), but level-0 local moving at 8G took 1.9–2.2× the in-RAM time, not ≤ 1.5× (times exploratory, shared machine). The inference, not tested, is that synchronous reads, not bytes, set the time: each thread waits for its block, then computes. So this cache is built with asynchronous read-ahead from the start.

**Scope**
- [ ] A cache in `everygraph-core`, pure logic over `read_at`, no OS calls and no `unsafe`:
  - fixed-size frames reserved from `Budget` as anonymous memory;
  - CLOCK eviction, sharded by block;
  - misses read the exact compressed byte range of a list, or a 4 KiB-aligned span of it, from the Elias–Fano offsets (D4);
  - frontier requests are batched, sorted and coalesced (the FlashGraph pattern). {{#m3-01-edge-map}} closed without it, so this issue owns it.
- [ ] Asynchronous read-ahead from the start (ADR 0029). Decided 2026-11-26 (`docs/research/sota-review-2026-11.md` S5):
  - **the base is a pool of reader threads over `pread`** behind `ByteSource` (in `everygraph-io`; the core stays free of OS calls). It takes batches of sorted ranges and merges neighbouring ones, with two caps: a largest merged read, and a largest amplification (bytes read that no request asked for, as a share of the bytes asked for). The queue depth comes from {{#m2-35-random-read-probe}}. The buffers in flight are reserved in the ledger;
  - **io_uring is an optional Linux backend** of the same interface. It is kept only if {{#m2-35-random-read-probe}} shows the pool below 90% of io_uring's IOPS at the queue depth the device needs. It falls back to the pool when `io_uring_setup` fails with `EPERM` or `ENOSYS` (Docker's and containerd's default seccomp profiles block it, as reported by the review). wasm32 keeps synchronous reads;
  - **prefetch follows the known visiting order.** ADR 0015's order is a fixed function of the graph and the seed, so the blocks of a whole sweep are known before it starts. The cache reads the next K blocks of that schedule with the pool's reads in flight; K is measured (1, 8, 64, 512 on m1-15's case), and pruned visits only skip blocks. Eviction stays CLOCK: by ACGraph's Fig. 2 (their machine), Belady's optimal policy cut reads by only 10–29% on sweeps, so a next-use eviction is optional
- [ ] Used by the random-access stages that need it first: the layout ({{#m5-01-layout}}), the tile builder ({{#m5-03-tile-builder}}), out-of-core Leiden level 0 ({{#m4-04-ooc-leiden}}) and out-of-core queries ({{#m7-03-ooc-queries}}). mmap stays for warm data (regime 1) and for scans
- [ ] ADR 0029 amends D11 (regime 2 = explicit cache for random access). It was accepted on 2026-11-26 on the bytes read of {{#m1-15-mmap-hints-block-cache}}; when the cache lands, DESIGN's D11 decision text changes from per-phase `MADV_RANDOM` to the cache and the planner rule in {{#m3-02-planner}} switches such phases from `MADV_RANDOM` to the cache

**Acceptance**
- The deterministic result hash is identical through the cache and through mmap on every oracle graph (the oracle's cross-source check)
- Resident memory of a cached run stays within the ledger plus the stated headroom under `MemoryMax`
- **Level-0 local moving at 8 GiB within 1.5× of in RAM**: M1-02's out-of-core Leiden on Friendster, async, one iteration, f64 weights, 16 threads, measured as in {{#m1-15-mmap-hints-block-cache}} (in-RAM reference in the same session, interleaved with plain mmap and per-phase `MADV_RANDOM`; the ratio decides, and the milestone window's official run gives the times, CLAUDE.md § 6)
- **The deterministic mode too** (the default, ADR 0014): deterministic Leiden level-0 local moving on Friendster at 8 GiB and at 4 GiB, cold, through the cache: the hash equals the in-RAM hash; bytes read and the number of reads per sweep are reported; time as a ratio against in RAM in the same session. All the evidence above is on the async mode. The deterministic mode visits sparse groups of ids sub-round by sub-round (1,024 sub-rounds per sweep on Friendster by the formula), so it makes many more small reads (an estimate, not measured), which is where queue depth matters most
- Bytes read for level-0 local moving and refinement at 8 GiB and 4 GiB no worse than the prototype's (62.5 GB and 87.3 GB with 4 KiB frames; `lab/m1-15-mmap-hints-block-cache/results/leiden_cache_pf.jsonl`), and time no worse than per-phase `MADV_RANDOM` in the same run

**Pointers.** `docs/adr/0029-random-access-under-a-budget.md`; the prototype `lab/m1-02-ooc-leiden/src/bcache.rs` and `lab/m1-15-mmap-hints-block-cache/results/README.md`; `docs/research/sota-io-memory.md` § 4 C1, C5, C7; `crates/everygraph-core` (`Budget`, `ByteSource`), `crates/everygraph-io` (`FileSource`, `MmapSource`).

**Depends on** {{#m1-15-mmap-hints-block-cache}}, {{#m2-02-io-layer}}, {{#m2-04-arrays-accountant}}
