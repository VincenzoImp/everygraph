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

**Evaluation**
- Task-based: can a viewer answer "which regions does X bridge", "what are the strongest ties of this community", "is this node a hub" in < 30 s? Try with 3–5 people
- Cost: bytes per tile, frame time on integrated GPU
- Honesty: every view states the edge weight shown vs omitted

**Decision it feeds.** Tile content model ({{#m5-02-tile-format}}) and viewer scope ({{#m5-07-edge-strategy}}).
