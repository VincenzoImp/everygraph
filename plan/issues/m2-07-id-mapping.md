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

**Acceptance**
- 65 M 42-byte hex keys: dictionary built in < 15 s at 16 threads, forward lookup < 1 µs warm
- 3.6 B endpoint mappings in < 60 s after the dictionary exists
- Bounded-memory build of a 500 M-key dictionary under a 4 GiB budget

**Pointers.** `lab/f1-query/src/bin/dict.rs`, FINDINGS § F1.
