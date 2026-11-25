---
title: Static export and hosting recipes
milestone: M5 · The map
labels: type:feature, area:map, P1, risk:low, size:S
---
**Why.** "Here is the map, open the link" is the feature that spreads. It must work from the cheapest static hosting.

**Scope**
- [ ] `everygraph map GRAPH --out FILE --reveal full|summary` and `m.export(path, reveal=…)` → `.egmap` file + `viewer.html` beside it; `reveal` is required, and leaving it out is an error that gives the two ways forward
- [ ] Recipes tested end to end: GitHub Pages (file-size limits and LFS caveats), S3/R2/GCS with range requests and CORS, a plain nginx config
- [ ] Offline bundle: a folder that opens from `file://` or a tiny local server
- [ ] Detail presets with estimated output size shown before export
- [ ] Embedding snippet (iframe and web component)

**Acceptance**
- A 4 M-node map published on a free static host opens on a phone with first view < 250 KB
- Documentation page with the three recipes verified

**Depends on** {{#m5-03-tile-builder}}, {{#m5-05-viewer-core}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The page a recipient sees, the embed and the share image are in `design/preview/style/``export.html`. At export the user chooses what the map reveals (`choices.md` C3): full (ids and the links shown) or summary only (regions and counts).
