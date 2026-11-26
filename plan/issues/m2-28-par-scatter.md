---
title: par: ranged and sorted scatter without unsafe
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P1, risk:medium, size:S
---
**Why.** Three production issues are about to port lab code that writes through raw pointers: the CSR build ({{#m2-08-csr-build}}), production Leiden ({{#m4-01-leiden-production}}) and parallel aggregation ({{#m4-03-parallel-aggregation}}). The lab Leiden (`lab/b2-leiden/src/main.rs`) uses `SyncPtr` for three access patterns. Abdi et al. (SPAA'24) classify them by how much safe Rust costs:
- *Block* writes, which the facade's `par::for_each_chunk_mut` already covers ("fearless", zero cost).
- *Ranged indirect* writes (each task writes the slice between two offsets): coarse targets by offset ranges, list records in the CSR build. Safe with a monotone-offset check whose overhead the paper calls "negligible".
- *Single indirect* writes: the `members` scatter with `AtomicUsize::fetch_add` positions. It also gives a **member order that depends on the schedule**, unless sorted later, which must never reach a result (D14).

The facade has no helper for the last two, which invites `unsafe` in exactly the crates that forbid it. Replacing `unsafe` with checks or synchronization cost 1.3× at 1 thread and 2.1× at 24 threads on average in that paper's benchmarks (their machine), but the ranged check alone is negligible there. Decided 2026-11-25 (`docs/research/README.md` R5).

**Scope**
- [ ] `par::for_each_range_mut(data, offsets, f)` in `everygraph-core::par`: checks that `offsets` increase and end within `data`, splits with `split_at_mut`, no `unsafe` (the core forbids it); chunk boundaries fixed by grain as the rest of the facade
- [ ] A deterministic sorted scatter for single indirect writes: a parallel sort of (key, id) pairs — deterministic because the keys are distinct, safe, O(n log n) where the scatter was O(n) — or a two-pass counting sort over fixed blocks where few buckets exist
- [ ] Documentation in the facade: which pattern each helper serves, and that `fetch_add` positions give a schedule-dependent order
- [ ] Several columns with the same offsets in one call (targets and weights, or property columns), flat and segmented; the input of the scatter sent block by block, so that a re-readable input is parsed in each pass and never held in memory, and so that a caller can place one wave of buckets at a time
- [ ] Exploratory interleaved A/B against the lab Leiden's `SyncPtr` member scatter on synthetic labels at lj's and orkut's node counts, 16 threads: the `SyncPtr` scatter, the safe helper and the parallel sort; hashes identical, time and peak heap per variant reported (`docs/research/sota-approx-rust.md` challenge 5)

**Acceptance**
- No `SyncPtr` or raw-pointer scatter in the production crates
- Outputs identical at 1, 3 and 8 threads (unit test) and at 1, 4 and 16 threads (A/B), equal to a sequential model
- The A/B above recorded under `lab/m2-28-par-scatter/results`. The measurement on real data, end to end, belongs to {{#m2-08-csr-build}}, which is the first user of the helpers (amended 2026-11-27: the lab Leiden aggregation on real level-0 labels moved there, with "the helpers are used by #35")

**Pointers.** `docs/research/sota-approx-rust.md` § 2 B, § 3 B ("Ranged indirect writes" row), § 4 challenge 5; Abdi, Posluns, Zhang, Wang, Jeffrey, SPAA 2024, Table 3.
