---
title: Compact tier for the adjacency: the coding chosen by the codec census
milestone: M2 · Format and ingest
labels: type:feature, area:format, P2, risk:medium, size:M
---
**Scope reduced (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S11).** This issue builds the compact tier that the codec census of {{#m1-16-codec-0124}} chooses, and nothing else. The varint + zstd block codec once written here is dropped: on uk-2005 it reached 2.4 bits per entry at 44 µs per random list, against BV's 2.1 bits at 768 ns (FINDINGS § C, shared machine), so it is slower than BV at random access by 57× and not smaller. No codec is built before the census reports.

**Why a compact tier.** Codec 0 stays the computing format (ADR 0004). A smaller tier is useful where the budget does not hold codec 0 but holds a smaller form: when the compressed form fits and the raw one does not, compressed is 1.2–1.9× faster for PageRank and 5.4–8.8× for WCC and BFS on Friendster (FINDINGS § M1-13). It would also give wasm a small form, which BV through webgraph-rs cannot (D5). On the best order of each graph, BV-style coding is still 22–27% smaller than codec 0 (`lab/m1-09-hierarchy-order/results/README.md`, exploratory).

**Possible outcomes of the census, and what this issue then does**
- A coding passes the census threshold (at least 15% below codec 0 on at least three graphs, tables included) and its lab decoder holds its budgeted case: specify it as a new block codec value in `docs/spec/eg-adjacency.md`, with reader, decoders, fuzz target, and a planner rule that chooses codec 0 or the compact tier from the budget. A tier, never the default.
- No coding passes: the compact tier is "BV through the webgraph feature" ({{#m2-15-webgraph-feature}}) for native builds, none for wasm, and this issue closes with that result.

**Acceptance** (on go)
- Bits per entry on the census graphs equal to the census within 1% (index included); random-list latency and single-thread decode rate against codec 0, interleaved A/B; round trip and fuzzing as in #30; the budgeted case faster than codec 0 under the same `MemoryMax`

**Depends on** {{#m1-16-codec-0124}}, {{#m1-09-hierarchy-order}}
