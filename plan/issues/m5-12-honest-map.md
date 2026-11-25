---
title: Honest maps: captions, artefacts and what distances mean
milestone: M5 · The map
labels: type:docs, area:map, P1, risk:low, size:S, lab-evidence
---
**Why.** A map is persuasive, and it can mislead. The lab listed how: distances between top-level regions mean little (the top quotient graph is nearly complete); derived regions look like communities; discs cover 1–3% of the plane; edges not drawn are not absent. The old prototype's strongest trait was saying such things explicitly.

**Scope**
- [ ] In the viewer: a persistent caption (what is shown, what is omitted, what position encodes), distinct styling for `aggregate` and `subcommunity` regions, an "about this map" panel with algorithm, parameters, modularity and provenance
- [ ] In the docs: "how to read an Everygraph map" with do/do-not examples; a section on community-detection limits (resolution limit, communities in random graphs)
- [ ] Export captions with screenshots
- [ ] A reviewer checklist for showcase maps before publication
- [ ] A "questions to ask of this map" page, linked from the plan card (added 2026-11-25, `docs/research/README.md` H10): barriers to network exploration come before the tool — setting goals, choosing structures and visual mappings (AlKadi et al., VIS'22, abstract only) — so the page starts from the analyst's goal and says which view answers which question

**Acceptance**
- No screenshot can be exported without its caption
- The documentation page is linked from the viewer

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The caption strip, the "About this map" panel and the distinct drawing of artefact regions are approved as shown in `design/preview/style/``viewer.html`; exported figures always carry the caption (`export.html`).
