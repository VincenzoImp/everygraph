---
title: Freedom-to-operate check on tile-pyramid graph maps
milestone: M0 · Foundations and decisions
labels: type:decision, area:map, P0, risk:medium, size:S
---
**Why.** Uncharted Software's Salt / Aperture-Tiles work (hierarchical community detection → recursive layout → data-tile pyramid, 2015–2017) was marked "patent pending" in 2015. Our map pipeline has the same shape. The research pass did not establish whether a patent was granted or is in force.

**Scope**
- [ ] Search Google Patents / Espacenet / USPTO for Uncharted Software and Oculus Info filings on multi-scale graph visualization and tile pyramids; record numbers, status, claims, jurisdictions and expiry
- [ ] Collect prior art that predates them: GMap (Gansner, Hu, Kobourov 2009–2010), GraphMaps (Nachmanson et al. 2015), treemap and circle-packing layouts, slippy-map tiling
- [ ] If a relevant granted patent exists: document how our pipeline differs, and get a short professional opinion before the public launch

**Acceptance**
- A written memo in `docs/legal/` with a clear go / change-design / seek-advice verdict

**Note.** This is not legal advice produced by the project; the outcome may require a patent attorney.
