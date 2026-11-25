---
title: Archival block codec for the adjacency (varint + zstd per block), measured on uk-2005
milestone: M2 · Format and ingest
labels: type:feature, area:format, P2, risk:medium, size:M
---
**Why.** FINDINGS § C measured varint + zstd blocks at 2.4 bits/entry on uk-2005 against 11.4 for the byte tier (shared machine). {{#m2-03-adjacency-codec}} reserves block codec values ≠ 0 and refuses them; its "web graph ≤ 12 / ≤ 3 bits/edge" criterion moves here. Exploratory reference from PR #157: web-google 21.92 bits/entry (data 20.72 + index 1.20), round trip equal — not comparable to uk-2005 because its ids have no locality; on web-google the "first neighbour as a separate varint" form would have been 0.23 bits smaller, the opposite of lj and orkut, so that trade depends on id locality.

**Scope**
- [ ] Codec value 1 in `docs/spec/eg-adjacency.md`: per-block LEB128 words compressed as one zstd frame; smaller blocks where random access matters; what byte offsets mean inside an archival block
- [ ] Pure-Rust zstd encoder (ruzstd is in the tree): its ratio against libzstd on uk-2005
- [ ] Reader, decoders and fuzz target extended; uk-2005 fetched through the dataset registry

**Acceptance**
- uk-2005 ≤ 12 bits/entry with the byte tier and ≤ 3 with the archival tier (both indexes included); random list on an archival block measured against the lab's 44 µs; round trip and fuzzing as in #30
