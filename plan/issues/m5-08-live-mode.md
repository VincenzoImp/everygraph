---
title: Live mode: the viewer talks to a local engine
milestone: M5 · The map
labels: type:feature, area:viewer, P1, risk:medium, size:M
---
**Why.** A static file answers what was precomputed. Investigation needs arbitrary follow-ups: full neighbourhoods, shortest paths between two selected nodes, filters by attribute or time.

**Scope**
- [ ] `eg open graph.eg` starts a server bound to **127.0.0.1 only** by default (binding elsewhere requires an explicit flag and prints a warning) and opens the viewer
- [ ] Endpoints: tiles, search, neighbours with paging, k-hop with a budget, shortest path, node/edge properties, view definition (filter) → recomputed overlays
- [ ] Requests carry budgets and timeouts; responses carry "truncated" flags
- [ ] Same viewer code as static mode, different data source
- [ ] Authentication token in the URL for multi-user machines

**Acceptance**
- Shortest path between two clicked nodes on twitter-2010 rendered in < 100 ms in RAM
- Port scan confirms no listener on non-loopback interfaces by default

**Depends on** {{#m5-05-viewer-core}}, {{#m7-01-point-queries}}, {{#m7-02-shortest-paths}}
