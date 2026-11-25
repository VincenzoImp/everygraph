---
title: Property columns for nodes and edges
milestone: M2 · Format and ingest
labels: type:feature, area:format, P0, risk:medium, size:L, lab-evidence
---
**Why.** Filters, weights, time windows and the map's attribute overlays all read properties. Lab result: reaching a property through an edge-id indirection makes weighted PageRank 1.7× (4 threads) to 2.1× (32 threads) slower on orkut — one kernel, one graph — and predicates 3.3–6.3× slower than a column aligned with the adjacency entries; storing every column twice (once per direction) wastes space once properties exceed 4 B/edge.

**Scope**
- [ ] Hot columns (weight, timestamp, type) aligned with each adjacency (ADR 0028, amending ADR 0009): **per-event** columns (a time, each row's weight under `duplicates="events"`) indexed by event offset (`docs/spec/eg-adjacency.md` § 3.1: the `.events` section, or the entry offset when the graph has none); **per-pair** columns (aggregates over a pair's events; the `edges` column of `duplicates="sum"`) indexed by entry offset
- [ ] Decide and record which aggregates are stored per pair and which are computed from the events when asked: the exact pair sum of a weight column (`docs/spec/graph-semantics.md` § 4.3), which every weighted kernel reads, is stored; multiplicity is read from the `.events` sequence or a per-pair column, measured both ways; kernels never read per-event columns
- [ ] Cold or wide columns stored once per edge behind an edge id; the edge id is a per-event column (indexed by event offset, `num_edges` values; ADR 0028), `u32` below 2^32 events and `u64` from there; the in-adjacency also carries the edge id
- [ ] Node property table aligned with dense node ids
- [ ] Column types: integers, floats, bool, timestamp, fixed-width binary, dictionary-encoded strings, decimal128, decimal256 (exact 256-bit token amounts; `docs/spec/graph-semantics.md` § 2.6, § 9 item 7); a reported sum widens as § 4.3 says (integer → decimal128(38, 0), decimal128(p ≤ 26, s) → decimal128(38, s), otherwise decimal256(76, s); overflow is `InvalidInput`)
- [ ] On-disk encoding compatible with Arrow buffers so columns can be exported zero-copy. Decided direction (2026-11-25, `docs/research/sota-ingest-query.md` § 3 D9, § 4 C4; `docs/research/README.md` B17): **each column section is a complete Arrow IPC file.** Schema; record batches whose value buffers start at 4 KiB offsets; no IPC compression; a fixed batch row count (≤ 2^31 − 1 elements and < 2 GiB bodies per batch); dictionaries in the same section. The table of contents (or a side table) records each value buffer's absolute offset, so kernels never parse FlatBuffers. LargeUtf8 or dictionary encoding for big string pools
- [ ] The framing is written by our own code (a few hundred lines) and pinned with golden bytes. arrow-rs's writer is not used for the stored bytes, so a change in it cannot change our files
- [ ] First, the one-hour check, no graph: a 2^27-entry f32 column in two batches plus a dictionary-encoded u8 column. `pa.ipc.open_file` on the slice of a memory-mapped `.eg`; buffer addresses inside the map and `% 4096 == 0`; round trip; identical bytes at 1 and 32 threads. Polars reads the same bytes
- [ ] Per-block min/max of time and weight columns (zone maps aligned to the ~64K-edge adjacency blocks), as one optional section, for the views of {{#m7-04-views}} and {{#m7-05-temporal}}
- [ ] Policy for which columns are hot (user hint, or first use by a kernel promotes a column)
- [ ] Null handling
- [ ] Column statistics recorded by the build in the table of contents for every numeric edge column, over the stored values: `count`, `null_count`, `nan_count`, `min`, `max`, `sum` (exact, rounded once to f64), and per graph the largest pair multiplicity; a column is clean when `nan_count = 0`, `min ≥ 0` and `sum` is finite (`docs/spec/graph-semantics.md` § 2.7)
- [ ] Edge-type column stored dictionary-encoded, `uint8` codes up to 256 values, `uint16` up to 65,536, otherwise plain text (`docs/spec/graph-semantics.md` § 2.6); reading it needs a public option, which belongs to the readers (#32, #33)

**Acceptance**
- Weighted PageRank on a per-pair weight column (indexed by entry offset; on orkut, which has no repeated pairs, the same as per event) within 15% of the unweighted kernel on orkut
- Columns round-trip from Parquet input to Arrow output with identical values
- Column sections are readable by `pyarrow` given the schema, without Everygraph

**Pointers.** `lab/f1-query/src/bin/props.rs`.
