---
title: Live mode: the viewer talks to a local engine
milestone: M7 · Queries and views
labels: type:feature, area:viewer, P1, risk:medium, size:M
---
**Why.** A static file answers what was precomputed. Investigation needs arbitrary follow-ups: full neighbourhoods, shortest paths between two selected nodes, filters by attribute or time.

**Scope**
- [ ] `everygraph open graph.eg` (the command is {{#m8-12-cli-rest}}'s; this issue adds the live endpoints to it) starts a server bound to **127.0.0.1 only** by default (binding elsewhere requires an explicit flag and prints a warning) and opens the viewer
- [ ] Endpoints: tiles, search, neighbours with paging, k-hop with a `limit`, shortest path, node/edge properties, view definition (filter) → recomputed overlays
- [ ] Requests carry `limit` and `timeout`, and the session a `memory` budget ("budget" means memory only, `docs/api-sketch.md` § 2.4, § 7); responses carry "truncated" flags
- [ ] Same viewer code as static mode, different data source
- [ ] Authentication token in the URL for multi-user machines

**Acceptance**
- Shortest path between two clicked nodes on twitter-2010 rendered in < 100 ms in RAM
- Port scan confirms no listener on non-loopback interfaces by default

**Depends on** {{#m5-05-viewer-core}}, {{#m7-01-point-queries}}, {{#m7-02-shortest-paths}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Same screens as the static viewer (`design/preview/style/viewer.html`); live mode adds answers the file cannot hold (full neighbourhoods, arbitrary paths, filters). The server prints the line shown in `cli.md` ("Serving on http://127.0.0.1:… (this computer only)").
