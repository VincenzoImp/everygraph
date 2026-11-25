---
title: Parquet and Arrow readers for edges and node tables
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P0, risk:medium, size:M
---
**Why.** Parquet is how large tabular data actually arrives (DuckDB, Spark and data-lake exports), it avoids the gzip bottleneck, and dictionary-encoded id columns can make id mapping much cheaper. Not tested in the lab.

**Scope**
- [ ] Read edge tables through `arrow-rs`/`parquet` with row-group parallelism and column projection
- [ ] Accept Arrow record batches directly (the Python path: Polars/pandas/DuckDB hand over batches zero-copy)
- [ ] Node tables: an optional table of node ids with property columns, including isolates
- [ ] Use dictionary pages to map each distinct key once ({{#m2-07-id-mapping}})
- [ ] Multi-file datasets (globs, hive-style directories)
- [ ] Keep `arrow`/`parquet` out of `everygraph-core`; check their wasm32 story for the browser tier and record it
- [ ] Edge-type column: reading a column beyond source, target, weight and time needs a public option, which has no name yet and needs a preview before it is built (`docs/spec/graph-semantics.md` § 2.6, § 9 item 8)

**Acceptance**
- Friendster as Parquet (integer ids) → `.eg` no slower than from plain text
- Round-trip of all supported column types into property columns

**Depends on** {{#m1-11-real-inputs}}
