---
title: Text and CSV edge-list reader
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P0, risk:low, size:M, lab-evidence
---
**Why.** Text is still the most common input. The lab parser reaches 0.72 GB/s per thread and 7–10 GB/s at 32 threads from the page cache, with no SIMD and no overflow checks.

**Scope**
- [ ] Parallel chunked parsing over `ByteSource` with page-aligned chunks and newline resynchronization
- [ ] Delimiter and header detection; comment lines; quoted fields; configurable column selection (source, target, optional weight, timestamp, type, extra columns)
- [ ] Typed fields: integer ids with overflow rejection (the prototype silently wraps), hex ids, string ids, floats, timestamps (epoch and ISO 8601)
- [ ] Compressed streams: zstd (parallel frames when available), gzip with a clear note that inflate is the bottleneck (0.3–0.6 GB/s, 10–20× slower than parsing) and a suggestion to convert
- [ ] Error policy: strict by default with line numbers; optional skip-and-count
- [ ] Two-pass capability (the source is re-readable) for bounded-memory builds and the browser

**Acceptance**
- ≥ 5 GB/s parse rate at 16 threads on plain text from page cache on the lab machine
- Malformed-input corpus handled without panics; every rejected line is counted and reported
- Identical output for plain, zstd and gzip variants of the same file

**Pointers.** `lab/b1-core/crates/glab-core/src/ingest.rs`.
