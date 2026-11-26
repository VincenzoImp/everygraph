---
title: "Research challenge: a navigable map of the Web Data Commons 2012 hyperlink graph on one machine"
milestone: M9 · After v0.1
labels: type:experiment, area:map, P2, risk:high, size:XL
---
**Why.** The page graph has about 3.56 B nodes and 128.7 B arcs (webdatacommons.org/hyperlinkgraph). Graph engines have computed on it; nobody has published a hierarchical, searchable, shareable map of it on one machine. It is a scientific challenge, not a release goal (owner's strategy discussion, 2026-11-26). Its 3.56 B nodes stay below 2^32, so it does not test 64-bit ids.

**Scope**
- [ ] Feasibility first: disk, state per node, hierarchy depth, hub degrees, coordinate precision ({{#m5-17-coordinate-precision}}), tile count and artefact size, each estimated before any run
- [ ] The host graph first (101 M nodes, 2.0 B arcs, in {{#m1-10-graph-families}}), then the page graph
- [ ] Report what the map can and cannot answer, with the quality metrics of m1-18

**Depends on** {{#m9-13-server-cartography}}
