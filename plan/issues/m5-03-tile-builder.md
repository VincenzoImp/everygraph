---
title: Tile pyramid builder: streaming, budgeted, deterministic
milestone: M5 · The map
labels: type:feature, area:map, P0, risk:medium, size:L, lab-evidence
---
**Why.** The lab builder produced correct pyramids (lj 8.5 s, orkut 15 s at 32 threads) but holds all tiles in RAM — fine for 851 MB, not for the 9–13 GB extrapolated for a 1 B-edge graph.

**Scope**
- [ ] Streaming build: generate tiles per quadtree subtree and append to the file; directory written at the end (PMTiles supports clustered, Hilbert-ordered tiles)
- [ ] Importance ranking per tile (region size, degree, PageRank) with stable tie-breaking
- [ ] Exact omission accounting per tile
- [ ] File size follows node count and budgets, not edge count (lab observation) — expose the size/detail trade-off as one "detail" parameter with a preflight size estimate
- [ ] Deterministic output: identical file hash at any thread count (lab: same SHA-256 at 4 and 32 threads)
- [ ] Incremental rebuild of subtrees when only part of the layout changed (stretch)

**Acceptance**
- lj pyramid ≤ 10 s at 16 threads with peak memory ≤ 1 GB
- A Friendster-size pyramid builds under an 8 GiB budget
- Same hash at 1, 4 and 16 threads

**Depends on** {{#m5-01-layout}}, {{#m5-02-tile-format}}
