---
title: Map file format (.egmap): tile content model on PMTiles v3
milestone: M5 · The map
labels: type:spec, area:map, P0, risk:medium, size:M, lab-evidence
---
**Why.** A published, stable map format is how other tools can read our maps and how maps outlive the engine. The lab proved a real PMTiles v3 file with a custom payload (`tile_type = 0`) works and is opened by the official Python reader; the official JS client was not tested.

**Scope**
- [ ] Tile content model for semantic zoom: at zoom z a tile holds the entities visible there — regions (typed: community / subcommunity / aggregate) with exact node count, internal edges, volume and cut; individual nodes with id, degree and tree path when zoomed in — chosen by importance with a per-tile budget
- [ ] **Zoom consistency in the content model** (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S12; Been, Daiches, Yap, TVCG 2006; Been, Nöllenburg, Poon, Wolff, CGTA 2010; MSAGL.js § 3.1). The lab selects per tile the top k by degree, which does not nest across zooms, and collision avoidance in the viewer would make labels depend on the frame. Instead ({{#m5-05-viewer-core}} draws what the tile says):
  - every entity (node and region) carries a minimum zoom `zmin`, a function of geometry and hierarchy only (its disc reaches a size in pixels; its parent region is open). An entity visible at zoom z stays visible at every deeper zoom;
  - labels are chosen at build time: per zoom, candidates in a fixed order (hierarchy depth, then size, then id), each accepted only if its box does not overlap the labels already accepted; each label stores its zoom interval [z_on, z_off). The builder uses the viewer's font metrics ({{#m5-14-design-system}});
  - the per-tile budget stays as a guard; what it drops is counted in "omitted", never dropped silently;
  - a popping count in a scripted dive (entities that appear, vanish and appear again) is part of the spec's test. Target: zero. If tiles at the deepest zoom pass 32 KB under this rule, the budget becomes nested (a parent tile's kept set is a subset of the union of its children's)
- [ ] Edge content per the outcome of {{#m1-06-edge-rendering}}; every tile states edge weight shown vs omitted and entities omitted
- [ ] Payload encoding: compact binary with a schema version. Compare three payloads: a custom layout, Arrow IPC, and MapLibre Tile (MLT; columnar, PMTiles tile type `0x06`, a Rust encoder, readers in MapLibre). Measure bytes per tile (gzip and not) and decode time in JS, and check that u64 counts and region kinds map to MLT columns without loss. MLT is adopted only if it is lossless for every field and within 10% of the custom payload's size and decode time; otherwise the custom payload stays (`docs/research/sota-review-2026-11.md` S12)
- [ ] The container stays PMTiles v3. MVT is not a payload option: it has no columns and no exact-integer guarantee for our counts (decided 2026-11-26)
- [ ] Sections beyond tiles: search index ({{#m5-04-search-index}}), per-node adjacency for on-demand ego edges, legend/metadata (graph name, counts, provenance, colour assignments), optional attribute summaries
- [ ] Where non-tile sections live: PMTiles metadata is JSON-only, so define an appended region that standard PMTiles readers ignore, and test with the official readers
- [ ] Budgets: median tile ≤ 8 KB gzip, max ≤ 32 KB (lab: 4–6 KB median, ≤ 22 KB max)
- [ ] Hostile input and declared sizes (added 2026-11-25, `docs/research/README.md` T6; `docs/research/sota-testing.md` challenges 1a and 10): the 32 KB budget bounds a tile's gzip size, not what it decodes to, so the spec also caps each tile's decoded size and bounds PMTiles run-length and shared-offset expansion of the directory. The danger is pointers that let several entries share bytes, not the codec: one DEFLATE stream cannot exceed a ratio of 1032, while overlapping entries in a zip container turned 42 kB into 5.5 GB (Fifield, WOOT'19). Readers allocate from no declared size before the bytes are read; a fuzz target for the tile payload ({{#m2-30-fuzz-ci}})
- [ ] What a `reveal="summary"` file holds, specified here before the format freezes (decided 2026-11-25, `docs/research/README.md` H1): no node-level entities, no per-node adjacency, no search index, no node attributes; regions under k nodes merged into `aggregate`; inter-region aggregates below k merged into "other" with consistent totals; provenance reduced to a label; attribute summaries and the time slider off unless asked; k in the metadata and the caption. The rules and their reconstruction test are {{#m5-15-summary-export}}; this issue makes sure the content model can express them (an "other" aggregate, a provenance label, k in the metadata)
- [ ] Spec in `docs/spec/egmap.md` with a reference reader in TypeScript and one in Python

**Acceptance**
- The official PMTiles JS and Python clients open an `.egmap` file and fetch tiles
- A third party could implement a reader from the spec alone (validated by the independent Python reader)

**Depends on** {{#m1-06-edge-rendering}}. If that experiment is late, this issue goes ahead on the approved selection-only design (links drawn on selection from per-node adjacency, nothing pre-drawn; Design reference below) and leaves room in the content model for edge aggregates.

**Do not implement** (decided 2026-11-26, `docs/research/sota-review-2026-11.md` § 4): MapLibre or deck.gl as the core renderer (deck.gl core alone is 219 KB min+gzip, MapLibre GL JS 300 KB, against the viewer budget of 150 KB of {{#m5-11-perf-accessibility}}; bundlephobia, 2026-11-26, as read by the review); MVT as the payload.

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** What a tile must carry follows from the approved viewer (`design/preview/style/viewer.html`, `data.html`): region kind (community / subdivision / aggregate), exact counts, the colour index of the top-level region from the five-colour assignment (computed once at build time by graph colouring of touching regions), label and count, shown-versus-omitted figures for the caption strip, and per-node adjacency for links drawn on selection. Links are not pre-drawn into tiles beyond what the selection needs.

**Patent note (`docs/legal/patent-check.md`, 2026-11-25).** Two granted patents have elements near a tile pyramid for graphs. Without drawing legal conclusions, prefer these engineering choices where they cost nothing: decide what a tile shows from the community hierarchy and exact aggregates rather than from a per-region quota of nodes ranked by importance; do not add shared edge-route segments carrying zoom levels; keep the index a plain tile directory, not a manifest of per-tile point counts negotiated against client resources. The specification from this issue is what goes to the attorney in {{#m8-10-legal-opinions}}.
