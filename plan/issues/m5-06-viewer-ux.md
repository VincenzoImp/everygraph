---
title: Viewer experience: inspect, search, colour, share
milestone: M5 · The map
labels: type:feature, area:viewer, P0, risk:medium, size:L
---
**Why.** The viewer is the product for the analyst persona. The lab prototype has hover and fly-to; a usable tool needs more, and it must keep the "exact numbers, declared omissions" discipline visible.

**Scope**
- [ ] Selection panel for a region or node: exact counts, strongest neighbours with weights, path in the hierarchy, node properties; "shown X of Y" wherever something is omitted
- [ ] Search box with exact and prefix search, keyboard navigation, fly-to
- [ ] Breadcrumb of the hierarchy and "zoom to parent / siblings"
- [ ] Colour modes: top-level region, any level, a node property, PageRank/degree; colour-blind-safe palettes; legend
- [ ] URL state: position, zoom, selection and colour mode encoded in the link, so a view can be shared
- [ ] Screenshot/export of the current view as PNG and SVG with caption and scale
- [ ] Empty/error states: slow network, unsupported browser, corrupt file
- [ ] Keyboard accessibility for all actions

**Acceptance**
- Usability check with 5 people from the target personas on three tasks (find a node, describe its community, find what connects two regions): ≥ 4 of 5 succeed without help
- Every number shown can be traced to a value in the file

**Depends on** {{#m5-05-viewer-core}}
