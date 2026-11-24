---
title: Attribute overlays, filters and a time slider on the map
milestone: M5 · The map
labels: type:feature, area:viewer, P2, risk:medium, size:L
---
**Why.** "Where are the exchanges", "show only edges above 10 ETH", "how did this look in March" are the questions an investigator asks of a map.

**Scope**
- [ ] Pre-aggregated attribute summaries per region and zoom in the map file (category counts, numeric quantiles) for static overlays
- [ ] In live mode: a view definition (attribute or time filter) recomputes region statistics and edge aggregates through the engine ({{#m7-04-views}}, {{#m7-05-temporal}})
- [ ] Time slider driven by time-partitioned aggregates
- [ ] Layout stays fixed across filters so the map remains a stable reference

**Acceptance**
- Colour-by-category on a 4 M-node static map with no extra requests beyond tiles
- Live filter round trip < 1 s on orkut

**Depends on** {{#m5-06-viewer-ux}}, {{#m5-08-live-mode}}
