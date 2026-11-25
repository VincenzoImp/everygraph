---
title: Summary export: k-threshold merging, stripped provenance, a reconstruction test
milestone: M5 · The map
labels: type:feature, area:map, P1, risk:medium, size:M
---
**Why.** `reveal=` is required when a map is written for sharing, with `"full"` or `"summary"` (`design/DECISIONS.md`, "What a shared map reveals"). The decision is right, but "summary" is not specified, and as the tile format stands it is not safe (`docs/research/sota-human-factors.md` § 3):
- tiles hold individual nodes (id, degree, tree path) and per-node adjacency ({{#m5-02-tile-format}}); without ids that is still a naively anonymized topology, and naive anonymization fails — Narayanan and Shmatikov re-identified 30.8% of the mapped users of an anonymized Twitter graph from Flickr data, at 12.1% error (S&P'09);
- exact counts with exact parent totals reveal any single hidden child by subtraction;
- provenance (path as given, modification time, input digest, fine inside a local `.eg`) becomes a leak if "About this map" copies it into a shared `.egmap`; m5-13's attribute summaries and time slider add channels.

Decided 2026-11-25, in v0.1, before the tile format freezes (`docs/research/README.md` H1).

**Scope**
- [ ] What a summary map holds, written into `docs/spec/egmap.md` ({{#m5-02-tile-format}}): no node-level entities, no per-node adjacency, no search index, no node attributes
- [ ] Regions with fewer than k nodes merged into the existing `aggregate` region kind — merging, not hiding a number, so "parent = sum of shown children" stays exact and nothing is left to subtract
- [ ] Inter-region edge aggregates below k merged into "other", with totals that stay consistent
- [ ] Provenance reduced to a label; attribute summaries and the time slider off unless asked for ({{#m5-13-attribute-overlays}})
- [ ] k written in the file's metadata and in the caption ("groups under k hidden"); its default and its name chosen here, recorded in `design/DECISIONS.md`
- [ ] The export path of {{#m5-09-static-export}} (`--reveal summary`, `reveal="summary"`) writes exactly this
- [ ] An adversarial test that tries to reconstruct any hidden count — a small region's size, a node's degree, an edge below k — from the file, by subtraction or from any section, and must fail

**Acceptance**
- The reconstruction test fails to recover any hidden count on every oracle graph and on lj
- A summary map of lj contains no node id, no per-node record and no provenance path, checked by reading every section of the file
- Every number shown in a summary map still adds up: each parent equals the sum of its shown children, "other" included

**Feeds.** {{#m5-09-static-export}}, {{#m5-12-honest-map}} (the caption), {{#m9-02-mcp-server}} (the reveal level applies to agent sessions), {{#m9-12-dp-map-export}}.

**Pointers.** `docs/research/sota-human-factors.md` § 2 ("Privacy of released graph summaries"), § 3 (the "What a shared map reveals" row), § 4 challenge 1, § 5.

**Depends on** {{#m5-02-tile-format}}, {{#m5-03-tile-builder}}
