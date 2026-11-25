---
title: Identifier dictionary: typed keys, minimal perfect hash, spilling dedupe
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P0, risk:medium, size:L, lab-evidence
---
**Why.** Real identifiers are addresses, usernames and URLs. The lab overturned the FST plan: on random keys an FST is 1.9× the raw key size and builds single-threaded (113–200 s for 65 M keys), while a minimal perfect hash built 500 M keys in 10.8 s at 2.61 bits/key with 0.56 µs lookups and 132 M mappings/s.

**Scope**
- [ ] Key typing by sampling: dense integers → rank bitmap (as in the lab ingest); sparse u64; hex/base58 → fixed-width bytes; UUIDs; arbitrary strings
- [ ] Distinct-key pass: hash-partitioned parallel dedupe that emits dense ids, with bounded memory by spilling partitions (lab: 15 M keys/s in RAM, spill untested)
- [ ] MPHF (`ptr_hash` or equivalent, checked for wasm32) + slot→id array + id-ordered key store; verified lookups; fixed-width keys need no offset array (24.3 B/key at 500 M)
- [ ] Layout variant with `[key | id]` in slot order so a cold forward lookup is one I/O (~80 µs) instead of three (~250 µs) — measure both
- [ ] Reverse lookup (id → key) from the id-ordered store
- [ ] Optional, lazily built front-coded sorted index for prefix/range search (3 µs lookups, 18 B/key on URL-like keys)
- [ ] Id assignment policy: by first appearance, by sorted key, or by a later renumbering ({{#m1-09-hierarchy-order}}) — the dictionary must survive renumbering
- [ ] Graph order — the order of `g.nodes()`, by which results are ordered and ties broken (`docs/api-sketch.md` § 2.2, § 2.12) — is fixed when the file is built and is the same for the same input and parameters at any thread count or budget; a later renumbering may change internal ids, never graph order
- [ ] Keys recognized as hexadecimal come back as lowercase text with the prefix the input used; look-ups accept any case
- [ ] An input whose distinct node count reaches 2^32 is refused with a clear error naming {{#m9-07-u64-ids-server-scale}}, never truncated (ADR 0003)

**Acceptance**
- 65 M 42-byte hex keys: dictionary built in < 15 s at 16 threads, forward lookup < 1 µs warm
- 3.6 B endpoint mappings in < 60 s after the dictionary exists
- Bounded-memory build of a 500 M-key dictionary under a 4 GiB budget
- The default id order is the natural one: dense integer keys in sorted order, any other key kind by first appearance in the input (the smallest input position wins, at any thread count and budget). Never hash order: whatever order ids get at ingest is the storage order, and a hash order stores every string-keyed graph at its worst size and locality (web-Google: 20.08 BV bits/edge in its natural random-like order against 7.69 after a BFS order, BP paper Table 2, their data; our web-google 21.9 bits/entry, PR #157). Tested on lj and web-google with string ids: bits/entry and PageRank time under first appearance and sorted key (`docs/research/sota-storage.md` § 3, challenge 4; recorded 2026-11-25)

**Pointers.** `lab/f1-query/src/bin/dict.rs`, FINDINGS § F1. Follow-up for the mapping loop: {{#m2-25-id-mapping-staged}} (`docs/research/sota-ingest-query.md` C2).
