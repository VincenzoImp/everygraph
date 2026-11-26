---
title: Viewer core: WebGL2 renderer and tile loader
milestone: M5 · The map
labels: type:feature, area:viewer, P0, risk:medium, size:L, lab-evidence
---
**Why.** The lab viewer shows a dependency-free WebGL2 page is enough for regions, nodes and edges with smooth zoom. It also showed what to fix: animated zooms fetch every intermediate level (up to 146 tiles per step) and there is no usable software-rendering path.

**Scope**
- [ ] TypeScript package with no runtime dependencies in the core; instanced rendering of regions, nodes and edges; text labels drawn as the tile says: the tile format ({{#m5-02-tile-format}}) stores labels chosen at build time with their zoom intervals, so the viewer only fades them in and out and runs no collision engine (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S12)
- [ ] The TypeScript reader of `docs/spec/egmap.md` (moved from {{#m5-02-tile-format}}): header, directories, section table, chunks and the tile payload as typed-array views; checked against `tools/egmap.py` on the samples of `tools/check_egmap_readers.py`. The payload is ours, not MapLibre Tile: the question is closed (`docs/spec/egmap.md` § 10)
- [ ] Tile loader over HTTP range requests with request coalescing, an LRU cache, cancellation of stale requests and throttling during animation
- [ ] Level-of-detail transitions (cross-fade parent → children) so zooming feels continuous
- [ ] Pluggable data source: static file, local file via `File` API, live engine ({{#m5-08-live-mode}})
- [ ] Adaptive budgets per frame from measured frame time ({{#m5-11-perf-accessibility}})
- [ ] Embeddable as a web component and as an ES module; no build step required to use it
- [ ] Hi-DPI, touch and trackpad gestures
- [ ] Styles, fonts, icons and the mark come from `@everygraph/style` (`design/system/`, {{#m5-14-design-system}}); the canvas reads the same tokens; no literal colour in the viewer's stylesheets (`design/system/tools/lint_colours.py` passes)

**Acceptance**
- First view ≤ 250 KB transferred and ≤ 25 requests on a 4 M-node map
- Zoom animation fetches ≤ 20 tiles per step
- 60 fps on a discrete GPU (the integrated-GPU and Safari budgets are checked in {{#m5-11-perf-accessibility}})

**Pointers.** `lab/e-map` viewer; GPU in headless Chromium needed `--use-angle=gl-egl`.

**Depends on** {{#m5-03-tile-builder}}, {{#m5-14-design-system}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The visual target is `design/preview/style/viewer.html` (navigable) with the tokens in `design/system/tokens.css`: dark canvas by default, bubbles, region outline 1.5 px with a tint of 12% / 5% when open / none when covering the view, labels with the count in monospace and a canvas-colour halo, smallest map text 12 px, selection as a double ink ring with the rest dimmed to 60%.
