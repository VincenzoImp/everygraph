---
title: Tile pyramid builder: streaming, budgeted, deterministic
milestone: M5 · The map
labels: type:feature, area:map, P0, risk:medium, size:L, lab-evidence
---
**Why.** The lab builder produced correct pyramids (lj 8.5 s, orkut 15 s at 32 threads) but holds all tiles in RAM — fine for 851 MB, not for the 9–13 GB extrapolated for a 1 B-edge graph. *Measured on Friendster 2026-11-25* ({{#m1-01-e2e-tracer}}, idle, 32 threads; `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 26, 27, 29; `lab/m1-01-tracer/results/tiles_unl_56G.json` and `logs/tiles-16g__chain-unl__graph-friendster__load-mmap__t32.rep1.err.tail`): with 56 GiB, 244.5 s and **29.1 GiB anonymous** for a 6.84 GB file (847,925 tiles, zmax 11); under 16 GiB, killed with the graph in the heap, and with it memory-mapped **no finish within the 60-minute timeout**. There 13.9 GiB was anonymous: the held tiles (gzip, about 2.55 GB through zoom 10) plus each zoom level's working state (candidate sets, per-tile hash maps, positions, order), leaving about 2 GiB for page cache, so each zoom level re-read the graph: 68–211 s per level for zoom 0–4 and 278–491 s for zoom 5–10, against 7–33 s with 56 GiB; zoom 11 never finished.

**Scope**
- [ ] Streaming build: generate tiles per quadtree subtree and append to the file; directory written at the end (PMTiles supports clustered, Hilbert-ordered tiles)
- [ ] Stream tiles to disk per zoom level (or per subtree), so resident memory does not grow with the number of tiles
- [ ] Bound the per-zoom working state as well (candidate sets, per-tile maps): on Friendster under 16 GiB it, not the held tiles, took most of the 13.9 GiB
- [ ] Compute all zoom levels from one adjacency pass rather than one pass per level (the lab re-reads the graph for each of the 12 levels)
- [ ] Never load the graph into the heap (killed at 13.2 s under 16 GiB that way)
- [ ] Importance ranking per tile (region size, degree, PageRank) with stable tie-breaking
- [ ] Exact omission accounting per tile
- [ ] File size follows node count and budgets, not edge count (lab observation) — expose the size/detail trade-off as one "detail" parameter with a preflight size estimate
- [ ] Deterministic output: identical file hash at any thread count (lab: same SHA-256 at 4 and 32 threads)
- [ ] Incremental rebuild of subtrees when only part of the layout changed (stretch)
- [ ] The pyramid is written as `NAME.egmap.partial` and renamed when complete; a stop removes the partial file (`docs/ux-contract.md` § 5.1, C-1, C-3)

**Acceptance**
- lj pyramid ≤ 10 s at 16 threads with peak memory ≤ 1 GB
- A Friendster-size pyramid builds under an 8 GiB budget; time and bytes read reported against the tracer's 244.5 s with 56 GiB (`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 29)
- Same hash at 1, 4 and 16 threads

**Depends on** {{#m5-01-layout}}, {{#m5-02-tile-format}}
