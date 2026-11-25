---
title: M5 epic: the map
milestone: M5 · The map
labels: type:epic, area:map, P0
---
Tracking issue for milestone M5.

**Goal.** Any graph becomes a zoomable map — regions → sub-regions → nodes — with exact numbers at every level, exported as one static file and explorable in a browser, a notebook or against a live local engine.

**Lab baseline.** lj (4.0 M nodes): layout 3.3 s, tiles 8.5 s, 173,017 tiles in an 851 MB PMTiles v3 file; first view 152 KB in 18 requests; full zoom-to-node session 5.0 MB; search in 2 requests; frames 16.67 ms on a discrete GPU; byte-identical output across thread counts. Weak points: artificial regions, edge fog, 97% of the area empty, no software-rendering path, tile builder holds everything in RAM.

**Exit criteria**
- [ ] {{#m5-01-layout}}, {{#m5-02-tile-format}}, {{#m5-03-tile-builder}}, {{#m5-04-search-index}}
- [ ] {{#m5-05-viewer-core}}, {{#m5-06-viewer-ux}}, {{#m5-07-edge-strategy}}
- [ ] {{#m5-09-static-export}}, {{#m5-11-perf-accessibility}}, {{#m5-12-honest-map}}, {{#m5-14-design-system}}
- [ ] {{#m5-15-summary-export}}, {{#m5-16-nonvisual-navigation}} (from the second literature review, 2026-11-25, `docs/research/README.md` H1, H2)

**Scope note.** M5 is the static map and depends only on M4. Three map features need other milestones and are tracked there: the Jupyter widget {{#m5-10-jupyter-widget}} in M6 (needs the Python API), live mode {{#m5-08-live-mode}} and attribute/time overlays {{#m5-13-attribute-overlays}} in M7 (need queries and views).

- [ ] A 1 B-edge graph opens from a static URL: first view < 250 KB, exact numbers at every level, no artificial region presented as a community

**Blocked by M1 results:** {{#m1-05-hierarchy-shaping}}, {{#m1-06-edge-rendering}}, {{#m1-07-layout-area}}, {{#m1-12-viewer-no-gpu}}; legal check {{#m0-10-patent-check}}.
