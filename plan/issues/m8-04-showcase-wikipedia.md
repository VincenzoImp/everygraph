---
title: Showcase map 1: the Wikipedia link graph
milestone: M8 · CLI, docs and v0.1 launch
labels: type:feature, area:launch, P1, risk:medium, size:M
---
**Why.** A public map everyone understands at a glance, with real labels (article titles), is the clearest demonstration of semantic zoom. English Wikipedia's link graph is on the order of 10^7 nodes and several 10^8 links — comfortably in range.

**Scope**
- [ ] Pipeline from the public dumps (pagelinks + page tables) to `.eg` with titles as ids and namespace/category as properties
- [ ] Communities, map hierarchy, region labels from top-PageRank article titles
- [ ] Published as a static map with search by title; a short write-up of what the map shows and what it does not
- [ ] Review with the checklist from {{#m5-12-honest-map}}
- [ ] Record full build time, peak memory and file size on a laptop-class budget

**Acceptance**
- Link opens on desktop and mobile; searching an article flies to it; build reproducible from a script in the repository

**Depends on** {{#m8-01-cli}}, {{#m5-04-search-index}}, {{#m5-12-honest-map}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Approved as the first public showcase (`choices.md` D3).
