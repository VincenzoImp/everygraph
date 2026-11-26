---
title: The build carries property columns: weights, times, pair aggregates and the degree profile
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P0, risk:medium, size:M
---
**Why.** {{#m2-08-csr-build}} closed on 2026-11-26 with its column boxes not built, and {{#m3-07-kcore-degrees}} left the stored degree profile to the build. Today `everygraph ingest --weight`/`--time` and `eg.read(weight=…, time=…)` refuse with `Unsupported`, so no weighted or temporal graph can be read. The column format exists ({{#m2-10-property-columns}}, `docs/spec/eg-columns.md`); what is missing is the build writing it.

**Scope**
- [ ] Carry property columns through the sort (weights, timestamps): per-event columns in event order, indexed by event offset (ADR 0028; `docs/spec/eg-columns.md`)
- [ ] Per-pair aggregates computed from each pair's event range (the exact pair sum of `docs/spec/graph-semantics.md` § 4.3) and written per pair, indexed by entry offset
- [ ] Record per graph the largest pair multiplicity (`docs/spec/graph-semantics.md` § 2.7); decide where it is stored and say so in `docs/spec/eg-columns.md` § 6
- The degree profile is not stored: it is computed on read from the entry offsets by {{#m6-09-api-gaps}}, and cached as a result when a summary needs it (decided in the review of PR #269; `docs/spec/eg-columns.md` § 6)
- [ ] The text reader's weight and time columns reach the build: `everygraph ingest --weight/--time` and `eg.read(weight=, time=)` stop refusing (`docs/api-sketch.md` § 3.1, § 5.2)
- [ ] Correct the doc comment of `AdjacencyIndex::entry_offset` (`crates/everygraph-format/src/adjacency/index.rs`): per-pair columns are indexed by it; per-event columns are indexed by event offset (moved from {{#m2-08-csr-build}})

**Acceptance**
- On every oracle graph with repeated pairs and a weight column, each pair's stored sum equals the sum of its events' weights, and each per-event column read back in event order equals the input's rows in (time, input order)
- The column statistics and the largest pair multiplicity equal values computed independently from the input (oracle)
- pyarrow and Polars read the written columns in place (`tools/check_columns_python.py`)
- Same bytes at 1, 4 and 16 threads, in RAM and with spills

**Pointers.** `plan/issues/m2-08-csr-build.md` (the boxes this issue takes over); `docs/spec/eg-columns.md` § 6 ("What the build stores"); `crates/everygraph-ingest/src/build/`.

**Depends on** {{#m2-08-csr-build}}, {{#m2-10-property-columns}}
