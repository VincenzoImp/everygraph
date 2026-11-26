---
title: Fuzz targets in CI: replay on every PR, short campaigns, nightly runs
milestone: M2 · Format and ingest
labels: type:infra, area:format, P1, risk:low, size:S
---
**Why.** The adjacency fuzz target exists (`crates/everygraph-format/fuzz/fuzz_targets/adjacency.rs`, raw and encode-then-damage modes), but no workflow runs it and no corpus is replayed, so the decoder's "never reads out of bounds under fuzzing" (m2-03) is a property of one afternoon, not of `main`. The container and the text reader have no target. Fifteen-minute campaigns in CI catch regressions, and triage skipped about 63% of fuzz sessions in 3 of 9 libraries (Klooster et al., SBFT'23). A CI fuzz run is a regression gate, not evidence of absence: claims about fuzzers need ≥ 30 trials of ≥ 24 h (Klees et al., CCS'18), which we do not make. Decided 2026-11-25 (`docs/research/README.md` T4, T6).

**Scope**
- [ ] Every PR: replay the committed corpus of every target as a normal test (`cargo fuzz run T corpus/T -- -runs=0`, seconds), inside the 15-minute budget of the PR CI
- [ ] PRs that touch a target's crate: 5 minutes per target, in parallel jobs, out of the required checks (nightly toolchain)
- [ ] Nightly, on the scheduled run: 1 h per target; new corpus entries come back by PR
- [ ] Targets: adjacency (exists); the container (`Container::open`, `verify_all`, `compact`, `Provenance::decode`, on raw bytes and in a decision-seed mode where `arbitrary` operations build a valid file and the tail of the input damages it — `docs/research/sota-testing.md` challenge 1h); the text reader (raw bytes, and gzip and zstd frames, including a zstd frame that declares a 128 MiB window); later the Parquet wrapper ({{#m2-06-parquet-reader}}) and the map file (`egmap`, added by {{#m5-02-tile-format}}: tile payloads, whole files, a damaged valid map)
- [ ] A regression test, under the counting allocator, that pins how the text reader uses ruzstd: a fresh `StreamingDecoder` does not pre-reserve the frame's declared window, but `DecodeBuffer::reset` does, so a decoder reused across frames would request up to 128 MiB per concurrently decoded frame outside the ledger (ruzstd 0.9.1, `frame_decoder.rs` l. 25 and 100–118, `decode_buffer.rs` l. 46–50; on wasm32 a real allocation against the 4 GiB space). The test fails if a decoder is ever reused
- [ ] Canary bugs kept behind a test-only flag (the historical untracked O(n) state; a reintroduced off-by-one in the codec); time-to-detect measured nightly, never cited as evidence

**Acceptance**
- The replay job is required on every PR and stays within the 15-minute budget
- The canaries are found by the nightly runs
- The ruzstd regression test exists and passes

**Left after PR #212 (2026-11-26).** PR #212 added the `container` and `text` targets, committed corpora, `tools/fuzz.sh`, `.github/workflows/fuzz.yml` (replay, 5-minute campaigns, nightly hour) and the ruzstd regression test. Still open:
- [ ] Codec canaries behind a test-only flag (a re-introduced off-by-one in the adjacency codec), and their time to detect, measured in the nightly run and never cited as evidence; the O(n) untracked-state canary belongs to {{#m3-10-no-oom-ci}}
- [ ] The nightly corpus comes back as an artifact; putting its new entries into a pull request is still done by hand

**Feeds.** {{#m2-03-adjacency-codec}}, {{#m2-05-text-reader}}, {{#m2-06-parquet-reader}}, {{#m5-02-tile-format}}, #28.

**Pointers.** `docs/research/sota-testing.md` § 3 (m2-03, m2-05, m0-02 rows), § 4 challenges 1h, 5 and 10; `.github/workflows/ci.yml`; `crates/everygraph-ingest/src/text/decompress.rs` l. 296–330.

**Depends on** {{#m0-02-ci}}
