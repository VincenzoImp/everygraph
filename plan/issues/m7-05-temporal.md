---
title: Temporal graphs: windows now, time-partitioned segments designed
milestone: M7 · Queries and views
labels: type:feature, area:query, P1, risk:medium, size:L, lab-evidence
---
**Why.** Transactions, messages and follows are events in time. Lab: sorting each neighbour list by time makes window scans 2–4× faster, but loses sorted-merge intersection and binary-search `has_edge`; the recommended design keeps the canonical adjacency neighbour-sorted and adds time-partitioned segments, which is also the route to incremental appends.

**Scope (v0.1)**
- [ ] Timestamp as a hot per-entry column; `g.window(t0, t1)` and `g.as_of(t)` as views ({{#m7-04-views}})
- [ ] Time-aware aggregates: edge counts and weight sums per time bucket for a node, a pair or a community
- [ ] Temporal degree and activity series as Arrow tables

**Scope (design only, implemented after v0.1 — {{#m9-04-temporal-segments}})**
- [ ] Segment format: append-only, time-partitioned adjacency segments; "as of T" is a union of whole segments plus filtered boundary segments
- [ ] Temporal path semantics (time-respecting paths) — define, do not implement

**Acceptance**
- Window PageRank on a 10% window of a temporal orkut ≥ 5× faster than the full graph
- Design note for segments reviewed and linked from the format spec

**Depends on** {{#m7-04-views}}
