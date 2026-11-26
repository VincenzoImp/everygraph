---
title: Honest maps: captions, artefacts and what distances mean
milestone: M5 · The map
labels: type:docs, area:map, P1, risk:low, size:S, lab-evidence
---
**Why.** A map is persuasive, and it can mislead. The lab listed how: distances between top-level regions mean little (the top quotient graph is nearly complete); derived regions look like communities; discs cover about 1–3% of the plane (estimate, not measured; DESIGN D17); edges not drawn are not absent. The map says such things explicitly.

**Scope**
- [ ] In the viewer: a persistent caption (what is shown, what is omitted, what position encodes), distinct styling for `aggregate` and `subcommunity` regions, an "about this map" panel with algorithm, parameters, modularity and provenance
- [ ] In the docs: "how to read an Everygraph map" with do/do-not examples; a section on community-detection limits (resolution limit, communities in random graphs)
- [ ] Export captions with screenshots
- [ ] A reviewer checklist for showcase maps before publication
- [ ] A "questions to ask of this map" page, linked from the plan card (added 2026-11-25, `docs/research/README.md` H10): barriers to network exploration come before the tool — setting goals, choosing structures and visual mappings (AlKadi et al., VIS'22, abstract only) — so the page starts from the analyst's goal and says which view answers which question
- [ ] The map states how strong its structure is: per level, modularity against the same graph with its edges shuffled at fixed degrees. A level whose communities are not clearly above the shuffle is marked weak in the map's metadata and in the viewer (graphs with little community structure, m1-10)

**Acceptance**
- No screenshot can be exported without its caption
- The documentation page is linked from the viewer

**Depends on** {{#m5-06-viewer-ux}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The caption strip, the "About this map" panel and the distinct drawing of artefact regions are approved as shown in `design/preview/style/viewer.html`; exported figures always carry the caption (`export.html`).
