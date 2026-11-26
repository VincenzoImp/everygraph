---
title: Out-of-core query path: pinned offsets, frontier prefetch, block cache
milestone: M7 · Queries and views
labels: type:feature, area:query, P1, risk:medium, size:L, lab-evidence
---
**Why.** Queries must stay useful when the graph is five times RAM. Lab findings: a cold 4 KiB fault costs ~55–90 µs at queue depth 1; pinning offsets removes one of two I/Os per neighbour access; prefetching the whole frontier with `MADV_WILLNEED` cut an orkut 2-hop from 8.7 to 0.47 ms; default readahead hurts single lookups (~2×) but helps BFS; under a 3 GiB cgroup on Friendster (15 GB) neighbours stayed at 105 µs p50.

**Scope**
- [ ] Offsets / block directory pinned in RAM (accounted), adjacency blocks through the block cache of {{#m2-23-block-cache}}; this issue builds no cache of its own
- [ ] Frontier-at-once prefetch API in the I/O layer; later an `io_uring` backend for true parallel submission
- [ ] Access-pattern hints per query type (random for point lookups, readahead for BFS)
- [ ] The same cache over a browser `ByteSource`, for the browser tier ({{#m9-01-browser-tier}})
- [ ] Honest limits: shortest path out of core is 25 ms p50 on 10^8-edge graphs and ~0.8 s on Friendster — document, and point to {{#m7-07-distance-index}}

**Acceptance**
- Cold neighbours p50 < 0.3 ms and 2-hop p50 < 1 ms on orkut and twitter-2010 on the lab NVMe
- Under `MemoryMax=3G` on Friendster: neighbours p50 < 0.15 ms, no OOM, throughput scales with concurrent queries

**Depends on** {{#m7-01-point-queries}}, {{#m2-02-io-layer}}, {{#m2-23-block-cache}}
