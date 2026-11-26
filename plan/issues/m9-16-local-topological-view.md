---
title: Local topological view: re-lay a selection by its own links inside the viewer
milestone: M9 · After v0.1
labels: type:feature, area:viewer, P2, risk:medium, size:M
---
**Why.** The map keeps communities inside regions, so two linked nodes can sit far apart (ADR 0017; Spearman about 0.03 on lj, m1-18). A selection (an ego network, a path's neighbourhood, a set of regions) could be laid out by its own links in a side view, while the map stays the orientation (owner's strategy discussion, 2026-11-26). Views (m7-04) and export to small-graph tools (m6-05) cover "extract", not "see it in place".

**Scope**
- [ ] A stress layout of the selected subgraph (up to a stated size), deterministic, with the selection's positions on the map kept as the start
- [ ] Brushing between the two views; the exact edge counts of the selection
- [ ] Static mode: only what the `.egmap` holds; live mode: the full neighbourhood

**Depends on** {{#m5-08-live-mode}}, {{#m7-04-views}}
