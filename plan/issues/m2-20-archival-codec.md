---
title: Archival block codec for the adjacency (varint + zstd per block), measured on uk-2005
milestone: M2 · Format and ingest
labels: type:feature, area:format, P2, risk:medium, size:M
---
**Why.** FINDINGS § C measured varint + zstd blocks at 2.4 bits/entry on uk-2005 against 11.4 for the byte tier (shared machine). {{#m2-03-adjacency-codec}} reserves block codec values ≠ 0 and refuses them; its "web graph ≤ 12 / ≤ 3 bits/edge" criterion moves here. Exploratory reference from PR #157: web-google 21.92 bits/entry (data 20.72 + index 1.20), round trip equal — not comparable to uk-2005 because its ids have no locality; on web-google the "first neighbour as a separate varint" form would have been 0.23 bits smaller, the opposite of lj and orkut, so that trade depends on id locality.

**Deferred (2026-11-25, `docs/research/README.md` B3; `docs/research/sota-storage.md` § 3 m2-20, challenge 6).** Not started before {{#m1-09-hierarchy-order}} and {{#m1-16-codec-0124}} report. On uk-2005, BV is both smaller and 57× faster at random access than varint + zstd blocks: 2.1 bits at 768 ns per random list, against 2.4 bits at 44 µs (FINDINGS § C, shared machine). Zuckerli is another 20–30% below BV on web graphs (Versari et al. 2020, their data). The byte tier loses on web graphs mainly through its 10-bit floor per gap, which an ordering and codec 2 attack. So:
- re-measure uk-2005 on the byte tier with the best order and codec 2;
- if that is within 2× of BV, the archival tier becomes "BV through the webgraph feature" ({{#m2-15-webgraph-feature}}) for native builds and is dropped for wasm, and this issue is closed with that result;
- zstd blocks are kept only if they remain the best wasm-portable archival option, with a random-list target in microseconds, not tens of microseconds.

**Scope**
- [ ] Codec value 1 in `docs/spec/eg-adjacency.md`: per-block LEB128 words compressed as one zstd frame; smaller blocks where random access matters; what byte offsets mean inside an archival block
- [ ] Pure-Rust zstd encoder (ruzstd is in the tree): its ratio against libzstd on uk-2005
- [ ] Reader, decoders and fuzz target extended; uk-2005 fetched through the dataset registry

**Acceptance**
- uk-2005 ≤ 12 bits/entry with the byte tier and ≤ 3 with the archival tier (both indexes included); random list on an archival block measured against the lab's 44 µs; round trip and fuzzing as in #30

**Depends on** {{#m1-09-hierarchy-order}}, {{#m1-16-codec-0124}}

**Input from m1-09 (#23, exploratory).** On the best order for each graph, BV-style coding is still 22–27% smaller than codec 0 (`lab/m1-09-hierarchy-order/results/README.md`), so the archival codec keeps its case after compaction.
