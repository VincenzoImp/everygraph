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
- [ ] The overview without selection, if {{#m1-06-edge-rendering}} adopts it after its preview under `design/preview/` (added 2026-11-25, `docs/research/sota-maps.md` challenge 5): a reordered matrix of the current parent's children with exact weights; a per-region external-weight glyph; "showing X% of inter-region weight" on the strongest-links layer. Built only to the approved preview and its row in `design/DECISIONS.md`

**Acceptance**
- The three evaluation tasks from {{#m1-06-edge-rendering}} are answerable in < 30 s on lj and orkut
- No view draws more edges than the per-frame budget; selection edges appear within 200 ms from a static file

**Depends on** {{#m1-06-edge-rendering}}, {{#m5-05-viewer-core}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Approved default: links are drawn only for the selection, as curved ink bands at 55% opacity with width by weight and exact figures in the panel; "strongest links always" is an optional layer; hierarchical bundling is dropped (`design/preview/style/SPEC.md`, `data.html` §4 and §6). The experiment in {{#m1-06-edge-rendering}} still has to show this answers the three user tasks on dense graphs.
