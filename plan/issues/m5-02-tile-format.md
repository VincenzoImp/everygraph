---
title: Map file format (.egmap): tile content model on PMTiles v3
milestone: M5 · The map
labels: type:spec, area:map, P0, risk:medium, size:M, lab-evidence
---
**Why.** A published, stable map format is how other tools can read our maps and how maps outlive the engine. The lab proved a real PMTiles v3 file with a custom payload (`tile_type = 0`) works and is opened by the official Python reader; the official JS client was not tested.

**Scope**
- [ ] Tile content model for semantic zoom: at zoom z a tile holds the entities visible there — regions (typed: community / subcommunity / aggregate) with exact node count, internal edges, volume and cut; individual nodes with id, degree and tree path when zoomed in — chosen by importance with a per-tile budget
- [ ] Edge content per the outcome of {{#m1-06-edge-rendering}}; every tile states edge weight shown vs omitted and entities omitted
- [ ] Payload encoding: compact binary with a schema version (compare a custom layout with Arrow IPC for size and decode cost in JS)
- [ ] Sections beyond tiles: search index ({{#m5-04-search-index}}), per-node adjacency for on-demand ego edges, legend/metadata (graph name, counts, provenance, colour assignments), optional attribute summaries
- [ ] Where non-tile sections live: PMTiles metadata is JSON-only, so define an appended region that standard PMTiles readers ignore, and test with the official readers
- [ ] Budgets: median tile ≤ 8 KB gzip, max ≤ 32 KB (lab: 4–6 KB median, ≤ 22 KB max)
- [ ] Hostile input and declared sizes (added 2026-11-25, `docs/research/README.md` T6; `docs/research/sota-testing.md` challenges 1a and 10): the 32 KB budget bounds a tile's gzip size, not what it decodes to, so the spec also caps each tile's decoded size and bounds PMTiles run-length and shared-offset expansion of the directory. The danger is pointers that let several entries share bytes, not the codec: one DEFLATE stream cannot exceed a ratio of 1032, while overlapping entries in a zip container turned 42 kB into 5.5 GB (Fifield, WOOT'19). Readers allocate from no declared size before the bytes are read; a fuzz target for the tile payload ({{#m2-30-fuzz-ci}})
- [ ] What a `reveal="summary"` file holds, specified here before the format freezes (decided 2026-11-25, `docs/research/README.md` H1): no node-level entities, no per-node adjacency, no search index, no node attributes; regions under k nodes merged into `aggregate`; inter-region aggregates below k merged into "other" with consistent totals; provenance reduced to a label; attribute summaries and the time slider off unless asked; k in the metadata and the caption. The rules and their reconstruction test are {{#m5-15-summary-export}}; this issue makes sure the content model can express them (an "other" aggregate, a provenance label, k in the metadata)
- [ ] Spec in `docs/spec/egmap.md` with a reference reader in TypeScript and one in Python

**Acceptance**
- The official PMTiles JS and Python clients open an `.egmap` file and fetch tiles
- A third party could implement a reader from the spec alone (validated by the independent Python reader)

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** What a tile must carry follows from the approved viewer (`design/preview/style/viewer.html`, `data.html`): region kind (community / subdivision / aggregate), exact counts, the colour index of the top-level region from the five-colour assignment (computed once at build time by graph colouring of touching regions), label and count, shown-versus-omitted figures for the caption strip, and per-node adjacency for links drawn on selection. Links are not pre-drawn into tiles beyond what the selection needs.

**Patent note (`docs/legal/patent-check.md`, 2026-11-25).** Two granted patents have elements near a tile pyramid for graphs. Without drawing legal conclusions, prefer these engineering choices where they cost nothing: decide what a tile shows from the community hierarchy and exact aggregates rather than from a per-region quota of nodes ranked by importance; do not add shared edge-route segments carrying zoom levels; keep the index a plain tile directory, not a manifest of per-tile point counts negotiated against client resources. The specification from this issue is what goes to the attorney in {{#m8-10-legal-opinions}}.
