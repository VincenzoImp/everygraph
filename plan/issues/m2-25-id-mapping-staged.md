---
title: Id mapping: a staged loop for billions of endpoints
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P2, risk:low, size:S
---
**Why.** Mapping every endpoint of the input to its dense id is a long loop: 3.6 B endpoints on Friendster, with a target of < 60 s after the dictionary exists ({{#m2-07-id-mapping}}). Today each lookup makes two dependent memory misses (pilot, then slot → id) and reads each `u32` through a `dyn ByteSource` call (`ids/dict.rs`). PtrHash's staged loop streams at 8 ns/key, DRAM-bound (PtrHash, SEA 2025, appendix A; i7-10750H, 10⁹ keys, their machine). It hashes a batch of keys to part and bucket, gathers the pilots, then the slots, then the slot → id entries, with no dependency between keys, on slices.

The review's estimate, unmeasured: up to about 3–4× on the 3.6 B mapping case. It compares two DRAM misses per endpoint (about 6–8 s of DRAM traffic) with the 132 M/s of one unverified run of the replaced crate, about 27 s (`docs/research/sota-ingest-query.md` § 3 D8 rows, § 4 C2).

**Scope**
- [ ] The staged loop: batches with no dependency between keys, on slices rather than per-element `dyn ByteSource` calls; safe Rust, wasm-safe; explicit prefetch only if callable without `unsafe`
- [ ] The MPHF solver's eviction scan starts at a pilot derived from a hash of (seed, bucket, eviction count) instead of pilot 0, with a 10·S limit as in PtrHash. It stays deterministic. Count evictions per part over 1,000 parts of 2^15 keys
- [ ] Ids unchanged: the same dictionary and the same mapping bytes as the per-key loop, at any batch size and thread count

**Acceptance**
- An interleaved A/B in one process: the per-key loop against batches of 64, 256 and 1,024; 500 M 20-byte keys and a 10⁹-endpoint stream; 1 and 32 threads. Report endpoints/s and `perf stat` L3 misses. The staged loop is kept only if it is faster at 32 threads
- 3.6 B endpoint mappings within {{#m2-07-id-mapping}}'s < 60 s target, measured through the harness

**Pointers.** `docs/research/sota-ingest-query.md` § 2.1, § 3 (D8 rows), § 4 C2 and C11 (PHastR noted, not adopted).

**Depends on** {{#m2-07-id-mapping}}
