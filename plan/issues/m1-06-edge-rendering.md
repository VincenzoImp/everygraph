---
title: Readable edges on the map: aggregation, on-demand ego edges, bundling
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:viewer, P0, risk:high, size:L, lab-evidence
---
**Question.** What should the map draw for edges at each zoom so that it informs rather than fogs?

**What the lab found.** Straight inter-region lines cross everything at mid zoom. Inside giant low-modularity communities node-level edges become fog. On orkut 90% of edge weight is not drawn at the deepest zoom (53% drawn on lj). The top-level quotient graph is nearly complete, so "who is connected to whom" carries little information there.

**Candidates to prototype in the lab viewer**
- [ ] Distance-adaptive aggregation: draw an edge only between entities of comparable screen size; aggregate the rest into per-entity "external weight" halos
- [ ] Significance filtering: show quotient edges whose weight exceeds the degree-corrected expectation (the enrichment measure the old prototype used for its matrix view) rather than the heaviest ones
- [ ] On-demand ego edges: click or hover a node/region → fetch its true neighbors from a per-node adjacency section (range request in static mode, engine call in live mode)
- [ ] Hub suppression: degree-capped drawing with an explicit "+k more" marker
- [ ] Hierarchical edge bundling along the community tree (Holten-style), computed per tile
- [ ] A matrix/arc alternative for dense regions

**An overview that needs no selection** (added 2026-11-25, `docs/research/sota-maps.md` § 3 D25 and challenge 5; `docs/research/README.md` B11). The approved design draws links for the selection only (D25), and that stays. But selection-only cannot answer "is it modular or all-to-all", and overview tasks dominate above 1,000 nodes (the survey of 152 studies, § V-A). Our top quotient is nearly complete. Additions to test:
- [ ] **Preview first.** These additions visibly change what the owner approved. Before any of them is prototyped in the lab viewer, a preview page under `design/preview/` (in the style of `design/preview/style/viewer.html` and `data.html`) shows them on a real map. The choice goes into `design/DECISIONS.md` (CLAUDE.md § 8: "show a preview first only when a choice would visibly change what the owner already approved")
- [ ] A reordered adjacency matrix of the current parent's children (or of the top level), with exact quotient weights, opened from the panel. Matrices beat node-link on dense graphs for every task except paths (Ghoniem et al., via Okoe et al. 2017). The ≤ 64–384 children's aggregated edges are already in the tile data the lab computes
- [ ] A per-region external-weight glyph: internal against external weight, and the top partner
- [ ] The "strongest links" layer always reads "showing X% of inter-region weight", and its bands are drawn so that they do not look like single edges. Bundling stays for the selection's link set only, along the hierarchy (Holten's own selection workflow)
- [ ] In the evaluation below, add the task "which regions are most tightly coupled", with and without the matrix

**Evaluation**
- Task-based: can a viewer answer "which regions does X bridge", "what are the strongest ties of this community", "is this node a hub" in < 30 s? Try with 3–5 people
  - *Moved (2026-11-26):* the sessions with people need participants, which M1 does not have. They run as the formative check of {{#m5-07-edge-strategy}} on the built viewer, with the fourth task added there. M1 answers the same tasks with deterministic checks instead: whether the information a task needs is on screen, and what share of the weight each view shows (`lab/m1-06-edge-rendering/results/overview_{lj,orkut}.json`).
- Cost: bytes per tile, frame time on integrated GPU
- Honesty: every view states the edge weight shown vs omitted

**Done when.** One approach is chosen with screenshots on lj and orkut, task-success numbers from the evaluation, and measured bytes per tile and frame time; rejected candidates are recorded with the reason. *Amended 2026-11-26:* task-success numbers from people move to {{#m5-07-edge-strategy}} (see above); frame times are exploratory, from headless Chromium on the RTX 4090 and on SwiftShader ({{#m1-12-viewer-no-gpu}}), and the official numbers come with the performance pass of M5.

**Decision it feeds.** Tile content model ({{#m5-02-tile-format}}) and viewer scope ({{#m5-07-edge-strategy}}).
