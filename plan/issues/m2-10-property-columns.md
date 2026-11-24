---
title: Property columns for nodes and edges
milestone: M2 · Format and ingest
labels: type:feature, area:format, P0, risk:medium, size:L, lab-evidence
---
**Why.** Filters, weights, time windows and the map's attribute overlays all read properties. Lab result: reaching a property through an edge-id indirection makes kernels 1.7–2.1× slower and predicates 3.3–6.3× slower than a column aligned with the adjacency entries; storing every column twice (once per direction) wastes space once properties exceed 4 B/edge.

**Scope**
- [ ] Hot columns (weight, timestamp, type) stored per adjacency entry, aligned with entry order
- [ ] Cold or wide columns stored once per edge behind a u32/u64 edge id; the in-adjacency also carries the edge id
- [ ] Node property table aligned with dense node ids
- [ ] Column types: integers, floats, bool, timestamp, fixed-width binary, dictionary-encoded strings, decimal128
- [ ] On-disk encoding compatible with Arrow buffers so columns can be exported zero-copy; evaluate Arrow IPC framing vs raw buffers + schema section
- [ ] Policy for which columns are hot (user hint, or first use by a kernel promotes a column)
- [ ] Null handling

**Acceptance**
- Weighted PageRank on a per-entry weight column within 15% of the unweighted kernel on orkut
- Columns round-trip from Parquet input to Arrow output with identical values
- Column sections are readable by `pyarrow` given the schema, without Everygraph

**Pointers.** `lab/f1-query/src/bin/props.rs`.
