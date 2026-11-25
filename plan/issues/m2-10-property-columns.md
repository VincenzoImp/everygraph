---
title: Property columns for nodes and edges
milestone: M2 · Format and ingest
labels: type:feature, area:format, P0, risk:medium, size:L, lab-evidence
---
**Why.** Filters, weights, time windows and the map's attribute overlays all read properties. Lab result: reaching a property through an edge-id indirection makes weighted PageRank 1.7× (4 threads) to 2.1× (32 threads) slower on orkut — one kernel, one graph — and predicates 3.3–6.3× slower than a column aligned with the adjacency entries; storing every column twice (once per direction) wastes space once properties exceed 4 B/edge.

**Scope**
- [ ] Hot columns (weight, timestamp, type) stored per adjacency entry, aligned with entry order
- [ ] Cold or wide columns stored once per edge behind a u32/u64 edge id; the in-adjacency also carries the edge id
- [ ] Node property table aligned with dense node ids
- [ ] Column types: integers, floats, bool, timestamp, fixed-width binary, dictionary-encoded strings, decimal128, decimal256 (exact 256-bit token amounts; `docs/spec/graph-semantics.md` § 2.6, § 9 item 7); a reported sum widens as § 4.3 says (integer → decimal128(38, 0), decimal128(p ≤ 26, s) → decimal128(38, s), otherwise decimal256(76, s); overflow is `InvalidInput`)
- [ ] On-disk encoding compatible with Arrow buffers so columns can be exported zero-copy; evaluate Arrow IPC framing vs raw buffers + schema section
- [ ] Policy for which columns are hot (user hint, or first use by a kernel promotes a column)
- [ ] Null handling
- [ ] Column statistics recorded by the build in the table of contents for every numeric edge column, over the stored values: `count`, `null_count`, `nan_count`, `min`, `max`, `sum` (exact, rounded once to f64), and per graph the largest pair multiplicity; a column is clean when `nan_count = 0`, `min ≥ 0` and `sum` is finite (`docs/spec/graph-semantics.md` § 2.7)
- [ ] Edge-type column stored dictionary-encoded, `uint8` codes up to 256 values, `uint16` up to 65,536, otherwise plain text (`docs/spec/graph-semantics.md` § 2.6); reading it needs a public option, which belongs to the readers (#32, #33)

**Acceptance**
- Weighted PageRank on a per-entry weight column within 15% of the unweighted kernel on orkut
- Columns round-trip from Parquet input to Arrow output with identical values
- Column sections are readable by `pyarrow` given the schema, without Everygraph

**Pointers.** `lab/f1-query/src/bin/props.rs`.
