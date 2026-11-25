---
title: Temporal graphs: windows now, time-partitioned segments designed
milestone: M7 · Queries and views
labels: type:feature, area:query, P1, risk:medium, size:L, lab-evidence
---
**Why.** Transactions, messages and follows are events in time. Lab: sorting each neighbour list by time makes window scans 2–4× faster, but loses sorted-merge intersection and binary-search `has_edge`; the recommended design keeps the canonical adjacency neighbour-sorted and adds time-partitioned segments, which is also the route to incremental appends.

**Scope (v0.1)**
- [ ] Timestamp as a hot per-entry column; `g.window(t0, t1)` as a view, closed at both ends, a date meaning its whole day (`docs/api-sketch.md` § 2.3); `as_of` is not public in 0.1 (`g.window(None, t)`) ({{#m7-04-views}})
- [ ] `window()` on a graph without a time column, or on a graph read with `duplicates="sum"` (the events' times are gone), is `Unsupported` (2), its way forward reading the input again with `duplicates="events"`; a start after the end is `ValueError` (2) (`docs/spec/graph-semantics.md` § 7, § 10 item 5)
- [ ] Time-aware aggregates: edge counts and weight sums per time bucket for a node, a pair or a community
- [ ] Temporal degree and activity series as Arrow tables
- [ ] Windows skip blocks through the per-block min/max of the time column ({{#m2-10-property-columns}}, {{#m7-04-views}}; added 2026-11-25, `docs/research/sota-ingest-query.md` C5). Many windows at once (Raphtory's `rolling` / `expanding`; Graphsurge's differential view collections, 1.5–13.9× over from-scratch on similar views, their machine) stay after v0.1, named here so the segment design leaves room for them

**Scope (design only, implemented after v0.1 — {{#m9-04-temporal-segments}})**
- [ ] Segment format: append-only, time-partitioned adjacency segments; "as of T" is a union of whole segments plus filtered boundary segments
- [ ] Temporal path semantics (time-respecting paths) — define, do not implement
- [ ] "As of" questions at many time points in one time-ordered pass (added 2026-11-25, `docs/research/README.md` D3; `docs/research/sota-dynamic.md` § 2.2, C5): on an append-only graph every earlier snapshot is a subset of every later one, so monotone kernels — WCC by union-find with min-id labels, reachability, BFS levels, degrees, event counts — give results at, say, 100 time points in one pass over the events in time order instead of 100 `window(None, t)` runs; for PageRank at k time points, Chronos-style batched state (k values per node, one scan per iteration), budgeted by the planner as k × n × 8 bytes. Published: Chronos > 10× single-threaded and > 2× on 16 cores over per-snapshot runs; CommonGraph 1.38–8.17× (their machines). Design here; the cheapest experiment, for whichever issue builds it: the WCC component count at 100 time points on a SNAP temporal graph, one sweep against 100 window runs

**Acceptance**
- Window PageRank on a 10% window of a temporal orkut ≥ 5× faster than the full graph
- Design note for segments reviewed and linked from the format spec

**Depends on** {{#m7-04-views}}
