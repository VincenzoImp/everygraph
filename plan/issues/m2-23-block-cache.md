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
  - frontier requests are batched, sorted and coalesced (the FlashGraph pattern already planned in {{#m3-01-edge-map}}).
- [ ] Asynchronous read-ahead from the start (ADR 0029): while a thread computes on one block, the misses of the next blocks in the visiting order are already being read, batched and coalesced into one request per run of neighbouring frames. Over `FileSource` `pread` with a small pool of reader threads, or `io_uring`; the choice is measured in this issue, not deferred
- [ ] Used by the random-access stages that need it first: the layout ({{#m5-01-layout}}), the tile builder ({{#m5-03-tile-builder}}), out-of-core Leiden level 0 ({{#m4-04-ooc-leiden}}) and out-of-core queries ({{#m7-03-ooc-queries}}). mmap stays for warm data (regime 1) and for scans
- [ ] ADR 0029 amends D11 (regime 2 = explicit cache for random access). It is proposed now and becomes accepted with the official `mmap-hints` run of {{#m1-15-mmap-hints-block-cache}}; when the cache lands, DESIGN's D11 decision text is updated and the planner rule in {{#m3-02-planner}} switches such phases from `MADV_RANDOM` to the cache

**Acceptance**
- The deterministic result hash is identical through the cache and through mmap on every oracle graph (the oracle's cross-source check)
- Resident memory of a cached run stays within the ledger plus the stated headroom under `MemoryMax`
- **Level-0 local moving at 8 GiB within 1.5× of in RAM**: M1-02's out-of-core Leiden on Friendster, async, one iteration, f64 weights, 16 threads, measured as in {{#m1-15-mmap-hints-block-cache}} (in-RAM reference in the same session, interleaved with plain mmap and per-phase `MADV_RANDOM`; official run)
- Bytes read for level-0 local moving and refinement at 8 GiB and 4 GiB no worse than the prototype's (62.5 GB and 87.3 GB with 4 KiB frames; `lab/m1-15-mmap-hints-block-cache/results/leiden_cache_pf.jsonl`), and time no worse than per-phase `MADV_RANDOM` in the same run

**Pointers.** `docs/adr/0029-random-access-under-a-budget.md`; the prototype `lab/m1-02-ooc-leiden/src/bcache.rs` and `lab/m1-15-mmap-hints-block-cache/results/README.md`; `docs/research/sota-io-memory.md` § 4 C1, C5, C7; `crates/everygraph-core` (`Budget`, `ByteSource`), `crates/everygraph-io` (`FileSource`, `MmapSource`).

**Depends on** {{#m1-15-mmap-hints-block-cache}}, {{#m2-02-io-layer}}, {{#m2-04-arrays-accountant}}
