# ADR 0008 — ID dictionary: typed keys and a minimal perfect hash

- **Status:** accepted, partly validated
- **Decision:** D8 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Real inputs name nodes with integers, hex strings (blockchain addresses) or arbitrary
strings (URLs, user names). Ingest must map billions of endpoint occurrences to dense ids
(ADR 0003) and results must come back with the original ids.

## Decision

Keys are typed. Integers map through a rank bitmap; hex keys are stored as fixed-width
bytes; strings use a minimal perfect hash (`ptr_hash`) plus an id-ordered key store; an
optional front-coded index gives prefix search. No FST by default (ADR 0000).

## Evidence

From the identifier experiment (FINDINGS § F1; shared machine):
- **MPHF at 500 M keys** (20-byte binary keys, 32 threads;
  `lab/f1-query/results/dict_mphf_ethbin_500000000_t32.json`): MPHF built in 10.8 s; the
  whole dictionary with its key store in 19.4 s (DESIGN's earlier "built in 11 s" was the MPHF
  alone; corrected on 2026-11-25); 2.61 bits/key (163 MB); warm forward lookup p50 0.56 µs; batch mapping in the
  ingest test 132 M lookups/s. FINDINGS marks the 132 M/s as unverified; DESIGN dropped that
  qualifier until on 2026-11-25.
- **FST at 65 M keys** (single-threaded build): 1.91× the raw key bytes on random binary
  keys (113 s), 1.03× on hex text (201 s), 0.28× on URL-like keys (53 s)
  (`lab/f1-query/results/dict_fst_ethbin_65000000_t4.json`,
  `lab/f1-query/results/dict_fst_eth_65000000_t4.json`,
  `lab/f1-query/results/dict_fst_user_65000000_t4.json`). The FST is the smallest form on
  URL-like keys; it loses on random keys and on build time.
- **Front-coded sorted blocks**: prefix search in about 3 µs at 18 bytes/key on URL-like
  keys (FINDINGS § F1; `lab/f1-query/results/dict_fc_user_65000000_t4.json`).
- **First dedupe pass**: about 15 M keys/s, which is about 4 minutes for 3.6 B endpoints —
  an extrapolation, not a measurement (FINDINGS § F1;
  `lab/f1-query/results/dict_dedupe_eth_65000000_t32.json`). Spilling was not tested.
- **Integer keys through a rank bitmap**: not measured in the lab.

## Consequences

- #34 builds the dictionary with a spilling dedupe; its acceptance (65 M hex keys in
  < 15 s at 16 threads, 3.6 B mappings in < 60 s, a 500 M-key build under 4 GiB) tests
  the parts the lab did not.
- The ingest bottleneck for string-id inputs is the dedupe pass, not the lookup; #25
  measures it on Friendster-shaped inputs.

## What would change our mind

- #25 or #34 show that the MPHF path cannot map 3.6 B endpoints in < 60 s, or cannot be
  built under a 4 GiB budget.
- String-id inputs with long shared prefixes turn out to dominate, and an FST built in
  parallel beats the MPHF plus key store on size by more than 2× at comparable build time.

## Amendment (2026-11-25, #34): the hash function is implemented in the workspace

The decision (typed keys, a minimal perfect hash, an id-ordered key store, no FST) stands;
the crate it named does not. `ptr_hash` 2.1.5, checked in the pull request of #34 and in
its review:

- **Its output differs from run to run.** Its global seed is fixed
  (`ChaCha8Rng::seed_from_u64(31415)`, `src/lib.rs`), but the solver's evictions draw from
  `fastrand::Rng::new()` (`src/build.rs`), which is seeded from a thread-local generator
  initialized from the clock and the thread's id. The same keys give a different
  function, and so different stored bytes, on every run. That conflicts with "What
  'repeatable' promises" (`design/DECISIONS.md`; `docs/api-sketch.md` § 2.11): the
  dictionary is part of the file whose sections are content-hashed, and the build must be
  the same at any thread count and budget. This is the strongest reason.
- **`cargo deny` fails.** `colored` (MPL-2.0, refused by `deny.toml`) is a dependency
  that no feature removes; with all features `rayon` also comes in outside
  `everygraph-core`, which `deny.toml` bans (`rayon` itself is optional).
- **Its stored form is not ours to specify.** With its `epserde` feature it is a zero-copy
  image of the crate's internal structures (behind `unsafe`); with `serde`, the same
  structures through a serializer. Either way the layout follows the crate's version.
- The wasm32 build fails without default features, because `rand` 0.10 pulls `getrandom`
  0.4, which needs its `wasm_js` feature on `wasm32-unknown-unknown`. That is probably
  one feature flag away, so it is a weak reason on its own. Without default features the
  crate still brings 38 other crates on that target, among them `tempfile`, `colored`,
  `rand` and three procedural-macro crates.

Decision: the same construction — skewed buckets (`PtrHash`'s cubic function), one 8-bit
pilot per bucket, 3.5 keys per bucket, load factor about 0.99, the slots past `n`
remapped into the holes below it — written in safe Rust in `everygraph-ingest`
(module `ids`), with parts of about 2^15 keys solved independently and deterministically
(each part is a function of its keys only: the ids are independent of threads and
budget, and so are the stored bytes, except that a budget too small for the bitmap of
dense integers gives the hashed form, with the same ids), and a stored form specified in
the module's documentation for readers. The key hash is XXH3-128, a published algorithm.

Dense integers: integer keys in sorted order are kept as a rank bitmap when every key is
in `0..2^32` (from the column's zero) and the largest key `m` satisfies `m + 1 ≤ 64 · n`,
so that the bitmap takes at most 8 bytes per key, less than the hashed form of 8-byte
keys (12.3 bytes per key: the key, slot to id, the hash function). Otherwise, and for
integers in first-appearance order, they go through the hash function as 8-byte keys.

Consequences:

- **The solver is ours to maintain** (about 550 lines with its tests, `ids/mphf.rs`):
  correctness was tested exhaustively for parts of 0–400 keys in the review, and the
  build retries the next hash seed when a part cannot be solved.
- **The format is ours**, and so is its compatibility: the `ids.*` sections are specified
  in the module documentation (enough for an independent reader) and versioned in
  `ids.header`.
- **Performance must be re-validated.** The lab's figures were measured with the crate,
  not with this implementation: until `bench/specs/id-dictionary.toml` has run officially,
  only the exploratory ones below exist.

Evidence (exploratory, shared machine, not to be quoted as performance): 2.61 bits per
key for the hash function at 2 M, 65 M and 500 M keys, as the lab's `ptr_hash` (2.61,
`lab/f1-query/results/dict_mphf_eth_65000000_t32.json`); 24.33 bytes per key for 20-byte
keys in the split layout, the figure #34 expected; a single-thread solver speed of
5.69 M keys/s on parts of 2^15 keys in the review (the lab's crate: about 1.45 M keys/s
per thread at 32 threads, whole build). The official measurements are the spec
`bench/specs/id-dictionary.toml`.

## Rules for files built by appends (2026-11-25, ADR 0028; for #111, not built in 0.1)

Segments do not exist yet; these rules are written now so that the dictionary and graph
order of 0.1 do not have to change when they arrive.

- **Graph order** in a file built by appends is the order of the base followed, for each
  segment in turn, by its new identifiers in the order that segment's input first names
  them. A fresh ingest of the concatenated inputs gives the same order only when ids are
  assigned by first appearance. A sorted-key or rank-bitmap order (dense integers) cannot
  place a new key between old ones without renumbering every id and every stored result.
- **The dictionary of a segmented file is layered:** the base dictionary plus, per
  segment, a dictionary of its new keys with ids from `n_{i−1}` (the node count before
  it); a lookup probes the layers newest first; a merge may rebuild one dictionary for
  the runs it merges. The minimal perfect hash is static, and rebuilding it on every
  append would cost O(n) per append.

## Closed by

#34 (dictionary with spilling dedupe), #25 (real-world inputs: string ids, Parquet,
compressed files).
