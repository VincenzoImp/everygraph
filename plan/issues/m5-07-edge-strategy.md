---
title: Edges on the map: production implementation
milestone: M5 · The map
labels: type:feature, area:viewer, P0, risk:high, size:L, lab-evidence
---
**Why.** Edges are the difference between a graph map and a bubble chart, and they are the weakest part of the lab prototype (fog inside giant communities; 90% of edge weight undrawn on orkut at the deepest zoom).

**Scope.** Implement the approach selected in {{#m1-06-edge-rendering}}; expected ingredients:
- [ ] Aggregated edges between entities of comparable screen size, ranked by significance rather than raw weight
- [ ] Per-entity "external weight" indicators instead of thousands of faint lines
- [ ] On-demand ego edges for the selected node or region from the per-node adjacency section (static) or the engine (live)
- [ ] Hub handling: degree-capped drawing with an explicit "+k more"
- [ ] Hierarchical bundling for the selected entity's edges
- [ ] Shown vs omitted weight displayed for every view

**Acceptance**
- The three evaluation tasks from {{#m1-06-edge-rendering}} are answerable in < 30 s on lj and orkut
- No view draws more edges than the per-frame budget; selection edges appear within 200 ms from a static file

**Depends on** {{#m1-06-edge-rendering}}, {{#m5-05-viewer-core}}
