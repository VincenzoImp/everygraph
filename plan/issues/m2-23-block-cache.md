---
title: Budgeted block cache over ByteSource for random access out of core
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P1, risk:medium, size:L
---
**Why.** Under a budget smaller than its working set, a random-access stage through a memory map re-reads the store many times: 733 GB for the tracer's layout on a 15.5 GB graph under 16 GiB (FINDINGS § M1-01). The page cache also makes resident memory differ from the ledger. An explicit cache over the portable `ByteSource` trait (D2) keeps the budget exact, works over `pread`, OPFS and HTTP ranges alike, and caches bytes, not results, so determinism is untouched (`docs/research/sota-io-memory.md` C1). This issue is built only if {{#m1-15-mmap-hints-block-cache}} shows the cache wins; otherwise it is closed with that result.

**Scope**
- [ ] A cache in `everygraph-core`, pure logic over `read_at`, no OS calls and no `unsafe`:
  - fixed-size frames reserved from `Budget` as anonymous memory;
  - CLOCK eviction, sharded by block;
  - misses read the exact compressed byte range of a list, or a 4 KiB-aligned span of it, from the Elias–Fano offsets (D4);
  - frontier requests are batched, sorted and coalesced (the FlashGraph pattern already planned in {{#m3-01-edge-map}}).
- [ ] `FileSource` `pread` misses now; an `io_uring` backend only once a measurement shows misses waiting on queue depth (C7, unchanged in {{#m2-02-io-layer}})
- [ ] Used by the random-access stages that need it first: the layout ({{#m5-01-layout}}), the tile builder ({{#m5-03-tile-builder}}), out-of-core Leiden level 0 ({{#m4-04-ooc-leiden}}) and out-of-core queries ({{#m7-03-ooc-queries}}). mmap stays for warm data (regime 1) and for scans
- [ ] The ADR amending D11 (regime 2 = explicit cache for random access), in the same pull request as the planner rule in {{#m3-02-planner}}

**Acceptance**
- The deterministic result hash is identical through the cache and through mmap on every oracle graph (the oracle's cross-source check)
- Resident memory of a cached run stays within the ledger plus the stated headroom under `MemoryMax`
- On M1-02's level-0 local moving at 8 GiB (Friendster), bytes read and time are reported against plain mmap, and are no worse than the prototype's in {{#m1-15-mmap-hints-block-cache}}

**Pointers.** `docs/research/sota-io-memory.md` § 4 C1, C5, C7; `crates/everygraph-core` (`Budget`, `ByteSource`), `crates/everygraph-io` (`FileSource`, `MmapSource`).

**Depends on** {{#m1-15-mmap-hints-block-cache}}, {{#m2-02-io-layer}}, {{#m2-04-arrays-accountant}}
