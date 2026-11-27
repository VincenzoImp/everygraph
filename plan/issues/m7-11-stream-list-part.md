---
title: "A native `list_part` for `Stream`: read only the entries asked for"
milestone: M7 · Queries and views
labels: type:feature, area:engine, P2, risk:medium, size:S
---
**Why.** `Stream` (`crates/everygraph-kernels/src/edge_map/source.rs`) has no `list_part` of its own. The trait's default reads the whole list from the `ByteSource` and returns the part. So every part of a list, and every prefix that a point query reads ({{#m7-01-point-queries}}, `crates/everygraph-query/src/graph.rs`), reads the whole list. On a hub, a query that needs a few entries reads millions, and a list read in `k` parts is read `k` times.

**Scope**
- [ ] `Stream::list_part` reads from the `ByteSource` only the bytes of entries `from..to` of list `v` (4 bytes each, at `4 * (offsets[v] + from)`), clamped to the list's end, into the reader's buffers, and checks them as `list` does
- [ ] `Stream::list_bytes` charges the part, not the whole list, when `part` is less than `entries`; the readers' no-OOM test (`crates/everygraph-memtest/tests/readers.rs`) still holds for `Stream`

**Acceptance**
- The query budget sweep (`crates/everygraph-query/tests/budget.rs`) and the kernels' oracle (`crates/everygraph-kernels/tests/oracle.rs`) pass on `Stream`
- For a hub, the bytes that the `ByteSource` reads fall in proportion to the part: a test counts the bytes read for parts of 1, 1/16 and the whole of a list of at least 2^20 entries, and finds each at most `4 * part` plus one page

**Depends on** {{#m7-01-point-queries}}
