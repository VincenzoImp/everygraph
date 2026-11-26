---
title: Search index inside the map file
milestone: M5 · The map
labels: type:feature, area:map, P1, risk:low, size:S, lab-evidence
---
**Why.** "Type an id and fly there" is what makes a map of millions of nodes navigable. Lab: a search index of 64 MB for lj answers a cold lookup in 2 range requests (47 KiB).

**Scope**
- [ ] Exact lookup: external id → (x, y, zoom, tree path) through a sparse index + sorted blocks, 2 range requests cold
- [ ] Prefix search for string ids using the front-coded sorted index from {{#m2-07-id-mapping}}
- [ ] Optional label search over region labels and a chosen node property
- [ ] Size budget: ≤ 20 bytes per node for exact lookup on integer or fixed-width ids

**Acceptance**
- Cold lookup ≤ 3 range requests and ≤ 64 KiB on a 100 M-node map
- Works from static hosting with no server code

**Depends on** {{#m5-03-tile-builder}}
