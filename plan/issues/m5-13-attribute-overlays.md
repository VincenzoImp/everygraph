---
title: Attribute overlays, filters and a time slider on the map
milestone: M7 · Queries and views
labels: type:feature, area:viewer, P1, risk:medium, size:L
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

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Colour by attribute and by metric, the legends and the time strip are drawn in `design/preview/style/viewer.html` and `data.html` (§3, §6, §8); when the fill is the datum it may be solid, unlike region tints.
