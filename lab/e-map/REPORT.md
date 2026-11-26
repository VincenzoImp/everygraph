# Experiment E — the graph map: hierarchical layout, tile pyramid, web viewer

All numbers below are from `bench.sh` run under `bench.lock` on the shared lab machine
(Ryzen 9 7950X, other experiments compiling/running: **noisy**), final binary, hierarchies
published by B2 in `/home/tux/graphlab/data/hier/{lj,orkut}` (7 levels each).
Raw files: `results/*.json`. The images named below (`img/`) were not committed; three of
them are in `docs/img/`.

## Verdict in five lines

1. **Feasible on one CPU box.** lj (35M edges): layout 3.3 s + pyramid 8.5 s at 32 threads. orkut (117M edges): 8.1 s + 15.0 s. CPU only, 0.9–2.3 GB RSS. The hierarchy itself is B2's cost and is not included.
2. **It reads well at the top and at the bottom, less well in the middle.** Continents and real communities are clear; derived intermediate levels and giant low-modularity communities produce artefacts (Q4).
3. **The exact numbers are exact.** Node count, internal edges and volume of 1,403,208 lj communities match B2's independently computed counts with 0 mismatches.
4. **A real PMTiles v3 file works as the single-file container** with a custom payload; the official Python `pmtiles` reader opens it. lj = 851 MB, orkut = 460 MB.
5. **Layout distances between continents carry little information**, and the "pull towards external neighbours" term gave no measurable gain. This is the main quality caveat.

## Q1 — Layout that scales and reads well

### Algorithm (`gmap/src/layout.rs`, "hcl")

Top-down over the community tree, one frontier (tree depth) at a time, parents in parallel:

- Every community is a **disc with area ∝ node count** (radius `R_parent·sqrt(φ·s_i/S)`, φ = 0.72; leaves φ = 0.5).
- For each parent, one scan of its members' adjacency gives the sibling quotient graph, the external pull of each child (mean unit vector towards the already-placed regions of external neighbours) and the parent's exact internal edge count.
- Children (≤ 384) are placed by a dense force-directed solve in the unit disc (surface-distance repulsion, attraction weighted `w_ij/sqrt(d_i·d_j)`, gravity calibrated to fill the disc, 50 iterations), then overlap-removal sweeps and a per-circle shrink that guarantees disjoint, contained circles.
- **Fan-out > 384 → derived intermediate level**: size-constrained label propagation on the sibling quotient graph groups children into "derived regions"; recursion continues inside them. This is needed constantly: B2's hierarchies have parents with up to 100,929 (lj) and 116,952 (orkut) children.
- Single-child chains are collapsed. Graph nodes are placed by the same solve inside their finest community.

Why circles: O(1) containment, the tile only needs (x, y, r), a bubble is one instanced quad in the viewer, and "bubble → its children" is a natural semantic-zoom step. The cost is wasted area (Q4). A power-diagram / Voronoi treemap would fill the plane and look more map-like; it was not attempted.

Complexity: `O(m·D)` for the scans (D = tree depth, 8–9 here) + `O(Σ_parents k²·iters)` for the solves with k ≤ 384, + `O(n·D)` bookkeeping. Memory: CSR + 28 B/node + 56 B/tree node.

### Cost (layout only, hierarchy given)

| graph | n | m | 1 thr | 4 thr | 8 thr | 16 thr | 32 thr | peak RSS | tree nodes (derived) | depth |
|---|---|---|---|---|---|---|---|---|---|---|
| lj | 4.00M | 34.7M | 23.1 s | 6.6 s | 4.6 s | 4.3 s | **3.3 s** | 0.88 GB | 1,413,237 (10,028) | 9 |
| orkut | 3.07M | 117.2M | 51.3 s | 17.5 s | 10.5 s | 11.3 s | **8.1 s** | 2.1 GB | 637,240 (19,986) | 8 |

- Target "lj < 2 min": met with a large margin, even single-threaded.
- **Thread scaling is poor: 7.0× (lj) and 6.3× (orkut) at 32 threads.** Likely causes: the apply pass is serial, the first frontiers have 1–37 parents, memory-bound scans, and a shared machine. CPU split at 1 thread: lj scan 6.3 s / solve 14.7 s; orkut scan 22.7 s / solve 24.5 s.
- **Determinism:** the first bench run *failed* this (f32 sums merged in a thread-dependent order). Fixed by merging chunk accumulators in a fixed order. Now `pos.f32` and `tnodes.bin` are byte-identical between 3 and 32 threads, and the `.pmtiles` file has the same SHA-256 at 4 and 32 threads, on both graphs (`results/determinism*.json`).
- Area retention after packing: 0.946 (lj), 0.955 (orkut) of nominal circle area.

**Extrapolation to ~1B edges (not measured).** From orkut's scan cost (~11 ns per adjacency entry per level) and the solve cost per node (3.7–8 µs): friendster (65.6M nodes, 1.8B edges) ≈ 350 core-s scan + 250–520 core-s solve, i.e. **roughly 2–4 minutes at 32 threads** given the scaling above, ~19–20 GB RAM (15 GB of that is the in-RAM CSR). *Measured since (#15, `docs/FINDINGS.md` § M1-01): 69.6 s (`total_s`), 17.7 GiB peak RSS, `lab/m1-01-tracer/results/layout_unl_56G.json`.* The serial fraction would dominate unless fixed; the scans could instead use B2's published quotient graphs for the upper levels.

### Quality: comparison

Metric: edge length in map units (map = unit square) relative to random node pairs. Lower = neighbours closer.

| layout | time @32 thr | median edge length | median ratio vs random pair | mean ratio | edges shorter than 1% of map |
|---|---|---|---|---|---|
| **lj, hcl** (ours, B2 hierarchy) | 3.3 s | 0.0061 | **0.014** | 0.226 | 53.2% |
| lj, hcl without external pull (ablation) | 3.3 s | 0.0063 | 0.015 | 0.223 | 53.1% |
| lj, hcl on my fallback hierarchy (label propagation, 5 s; 4-thread dev run) | — | 0.0183 | 0.045 | 0.371 | 46.3% |
| lj, FastRP-like embedding (8 dims, 4 rounds) + PCA → 2D | 0.95 s | 0.0346 | 0.244 | 0.264 | 12.9% |
| orkut, hcl | 8.1 s | 0.0329 | 0.074 | 0.285 | 32.4% |

- The embedding baseline is a featureless blob at every zoom (`img/contact.png`, bottom row). It is about 3× faster and useless as a map.
- **The external-pull term does nothing measurable** (0.226 vs 0.223 mean ratio). Honest negative result.
- **Hierarchy quality matters more than layout tuning**: the cheap fallback hierarchy gives a 3× worse median ratio.
- The mean ratio is dominated by inter-continent edges: 29% of lj edges and 42% of orkut edges are longer than 5% of the map. Top-level placement barely helps because the top-level quotient graph is almost complete.

## Q2 — Tile pyramid and single-file format

### Content model (`gmap/src/tiles.rs`, payload "glab-map-tile-v0")

World = unit square, 256-px tiles, zoom 0..zmax (zmax chosen so the 10th-percentile node radius is ≥ 4 px: 10 for lj, 9 for orkut). Per tile:

- **Bubbles** whose centre is in the tile and whose radius is 3–384 px at that zoom, top 300 by node count. Each carries id, parent, x, y, r, exact node count, internal edges, volume, hierarchy level, flags (derived / finest), colour id.
- **Context bubbles**: larger ones that intersect the tile (the "you are inside" chain).
- **Nodes** once their radius is ≥ 1.2 px, top 400 by degree (all of them at zmax): dense id, original id, x, y, r, degree, finest community.
- **Aggregated edges**: every graph node is mapped to its representative at this zoom (itself if shown, else its deepest shown ancestor). Edges between representatives are counted exactly; the tile keeps the top 600 by weight (ties: shorter first) with the far endpoint's id and position inline.
- **Honesty fields**: omitted bubbles (count and node sum), omitted nodes, total / shown edge weight, and weight going to non-displayed context.

### Measured (32 threads; 4 threads in brackets)

| | lj | orkut |
|---|---|---|
| build time | 8.5 s (22.5 s) | 15.0 s (51.0 s) |
| peak RSS | 2.2 GB | 2.3 GB |
| tiles | 173,017 | 63,846 |
| file size | **851 MB** | **460 MB** |
| – tile data (gzip) | 787 MB (1.98 GB raw) | 410 MB (1.07 GB raw) |
| – directories | 386 KB | 141 KB |
| – search index | 64.0 MB | 49.2 MB |
| tile bytes gz: median / p99 / max | 4.2 KB / 16.4 KB / 20.5 KB | 5.9 KB / 18.0 KB / 21.6 KB |
| share of incident edge weight shown: z0, z8, deepest | 67%, 12%, 53% (z10) | 72%, 4%, 10% (z9) |

- File size follows **node count and budgets, not edge count**: orkut has 3.4× the edges and a smaller file. About 197 B/node (lj) and 134 B/node (orkut) of tile data; the deepest zoom is about half the file.
- **The edge budget is the real loss**: at orkut's node level only 10% of the edge weight touching a tile is in the tile. The rest is counted, not drawn.
- **Extrapolation to friendster scale (65.6M nodes, 1.8B edges; not measured):** ~9–13 GB of tiles + 1.05 GB search index, 1–3.5M tiles, max tile unchanged (~22 KB, set by budgets). Build is `O(m × zoom levels)`: ~4–6 min at 32 threads by linear scaling from orkut. *Measured since (#15, `docs/FINDINGS.md` § M1-01): 243.9 s (`total_s`) for a 6.84 GB file, `lab/m1-01-tracer/results/tiles_unl_56G.json`.* The current builder holds all tiles in RAM before writing; that must become a streaming write. (#69)

### Container

- **Real PMTiles v3 can be reused.** Header `tile_type = 0` (unknown), gzip tiles, gzip directories, Hilbert tile ids, root + leaf directories. Verified with the official Python `pmtiles` reader on both final files (header, metadata, tiles at z0, z3, z6 and zmax, i.e. through leaf directories). The viewer uses its own ~60-line JS reader, so the official JS client is *not* tested.
- **Search index** is appended after the tile-data section (PMTiles readers ignore it) and referenced from the metadata JSON: records `(original id u64, x f32, y f32)` sorted by id, 16 B/node, plus a sparse top index (one key per 2,048 records; 15.6 KB for lj). Lookup = 2 range requests cold (47 KiB), 1 warm (32 KiB). The record has no per-node zoom; the viewer flies to zmax.
- **Directory of files instead:** computed, not written to disk — 1.22 GB on 4 KiB blocks for lj (+55% over the tile bytes) and 173k inodes; 563 MB for orkut. The single file is better for sharing and no worse for access.

## Q3 — Viewer

`viewer/` — plain WebGL2, one HTML + one JS file, no build step, no dependencies. Instanced quads for bubbles, nodes and edges; canvas-2D labels; hover tooltip with exact numbers and the tile's omitted figures; search box that flies to a node. Served by `viewer/serve.mjs` (Range support, binds 127.0.0.1 only).

Playwright, headless Chromium, 1600×1000 (`results/viewer_*.json`, screenshots `img/viewer-*.png`):

| | lj | orkut |
|---|---|---|
| initial load (header + root dir + metadata + 16 tiles) | 152 KB, 18 requests | 143 KB, 18 requests |
| each fly-to zoom step (4 steps, z1.9 → z10.6) | 0.66–1.86 MB, 94–146 tiles | 0.23–1.60 MB, 44–140 tiles |
| pan 800 px at node level | 0 B (already cached by the 1-tile margin) | 0 B |
| search, cold | 2 requests, 47.3 KiB | 2 requests, 43.7 KiB |
| whole scripted session | 5.0 MB, 546 requests | 3.9 MB, 454 requests |

- Zoom steps are expensive because the fly-through loads every intermediate zoom level; loading should be suspended during animation.
- **GPU:** real acceleration was obtained with `--use-gl=angle --use-angle=gl-egl` (renderer: RTX 4090, OpenGL ES 3.2). Frame interval 16.67 ms mean, 16.8 ms max, 0 frames over 33 ms in every zoom/fly test on both graphs. This is **vsync-capped**, so it shows "no dropped frames", not how much headroom there is.
- `--use-angle=vulkan` also reports the 4090 but loses the WebGL context in headless mode (blank canvas); not usable.
- **Software rendering (SwiftShader, lj) is not smooth:** mean frame 29–136 ms, worst 567 ms; at node level every frame is over 33 ms. The viewer needs a GPU (integrated GPUs untested) or lower per-frame edge counts.

Recommendation: stay with plain WebGL2 instancing. The workload is four instanced draw calls; deck.gl would add a dependency and a build step for no capability that was missing here.

## Q4 — What does not work / where the map misleads

- **Distances between continents mean little.** The top-level quotient graph is nearly complete, and the external-pull ablation shows no gain. Adjacency inside a parent is weakly meaningful; position across parents is not.
- **Circle nesting wastes space.** With 8–9 nested levels, node discs cover about 1–3% of the map. Zoom range is 10–11 levels and the overview is mostly empty space.
- **Derived regions are layout artefacts that look like communities.** lj has 10,028, orkut 19,986. Inside a giant finest community they come out at near-identical sizes (many "627 nodes" bubbles in `img/viewer-lj-4-communities.png`) — that regularity is the size cap, not graph structure. They are flagged and drawn fainter, which is not enough.
- **Giant low-quality communities become hairballs.** B2's lj level 0 has a 100,929-node community. The lj zoom target is a derived region there with 48 nodes, 199 internal and 5,512 cut edges; the node-level view is fog (`img/viewer-lj-5-nodes-edges.png`). The orkut target, a real 68-node community with 747 internal edges, is readable (`img/viewer-orkut-5-nodes-edges.png`).
- **Raw edges are unusable above node level** (`img/lj-hcl-1-zoom-rawedges.png`). The viewer draws only the 220 strongest aggregated edges in view; these are mostly links between sibling sub-bubbles, so inter-continent structure is not shown at all.
- **Node positions inside a finest community are near-uniform**; internal structure is visible only through edges.
- **Budgets cause popping and bias.** Lower-degree nodes appear one or two zoom levels after their neighbours; the per-tile edge cut favours short edges.
- Bubble area encodes node count only. A dense and a sparse community of the same size look identical.

For readable node-level edges the format would need: distance-adaptive aggregation (far endpoints collapsed to the sibling-of-common-ancestor region, drawn as counted stubs), a per-node adjacency range index so a hovered or selected node can fetch its full edge list, hub suppression by default, and edge bundling along the hierarchy.

## What failed or deviated

- Determinism failed on the first full run; fixed and re-measured (Q1).
- My igraph Louvain bootstrap for lj (212 s, levels 245,809 → 1,929) was used during development only. The orkut bootstrap never produced output (it exited within a minute of getting `bigmem.lock`; cause not established). All reported numbers use B2's hierarchies.
- `gmap hier` (fallback hierarchy by size-constrained label propagation) was run once on lj: 5.2 s, works, clearly worse map (Q1 table).
- The directory-of-files container was computed, not measured.
- Exact counts of derived regions have no independent reference; only the 1.4M real lj communities were cross-checked. orkut was not cross-checked.
- **Process incident (lab rule 5):** at about 13:01 I ran a `pgrep -f` pattern kill meant for my own wait loop; it matched and killed eight PIDs. Three (3690693, 3690735, 3701197) predate my loop and were almost certainly other experiments' shell commands. Their agents will have seen a failed command. No data was touched.

## Reproduce

```
flock /home/tux/graphlab/bench.lock ./bench.sh          # ~8 min incl. lock wait; writes results/*.json, out/*.pmtiles
py/.venv/bin/python py/pick.py out/lj-hcl /home/tux/graphlab/data/csr/lj > out/lj-hcl/target.json
py/.venv/bin/python py/make_images.py lj               # img/lj-*.png, img/contact.png
node viewer/serve.mjs . 8765 &                         # 127.0.0.1 only
(cd viewer && node capture.mjs lj ../img/viewer-lj ../results/viewer_lj.json ../out/lj-hcl/target.json)
# open http://127.0.0.1:8765/viewer/index.html?file=../out/lj.pmtiles
```
`bench.sh` needs no network. The image and viewer steps need the local venv and `viewer/node_modules` (already installed). The server is not left running.

## Recommendations

**Layout algorithm and complexity.** Hierarchy-constrained multilevel disc packing as implemented: `O(m·D + Σ_parents k²·iters)` time with fan-out bounded at ~384 by derived levels, `O(m + n)` memory, deterministic once merges are ordered. Before productising: drop or redesign the external-pull term, parallelise the apply pass, read upper-level quotient graphs from the community-detection output, and prototype a power-diagram variant to recover the wasted area. Require a good hierarchy: bounded community size at the finest level matters more than any layout parameter.

**Tile / container format — draft spec outline ("glab-map v0").**
1. Container: PMTiles v3, `tile_type = 0`, gzip (evaluate zstd) tiles and directories, clustered.
2. Metadata JSON: format/version, n, m, zmax, tile size, per-tile budgets, bubble pixel thresholds, colour table, search-section descriptor, hierarchy level names.
3. Tile payload: fixed header (zoom, x, y, counts, omitted bubbles/nodes, total/shown/context edge weight); bubble records (44 B); node records (36 B); edge records (20 B, far endpoint inline). Next version: tile-local u16 coordinates and columnar layout (smaller; gain not measured).
4. Semantic-zoom rules: visibility by on-screen radius, importance by size/degree, representative mapping, exactness guarantees and the meaning of every "omitted" field.
5. Search section after tile data: sorted `(id, x, y[, zoom])` records + sparse top index; later a name/label index.
6. Optional sections: per-node adjacency ranges, per-community property summaries.

**Measured costs.** lj: layout 3.3 s, tiles 8.5 s, 851 MB, 173,017 tiles. orkut: layout 8.1 s, tiles 15.0 s, 460 MB, 63,846 tiles. Both at 32 threads on a shared machine, ≤ 2.3 GB RSS, max tile 21.6 KB. Viewer: 152 KB to first view, ~5 MB for a full zoom-to-node session, search 47 KiB.

**Viewer technology.** Dependency-free WebGL2 with instancing; throttle tile loading during animated zooms; cap edges per frame for machines without a discrete GPU.

**Biggest remaining risks.**
1. The middle zoom levels depend on derived regions that have no meaning to the user.
2. Edges: at orkut density 90–96% of edge weight is omitted at deep zoom; the feature may read as "bubbles with counts" rather than "a map of the graph".
3. All 1B-edge figures are extrapolations; layout thread scaling (7× on 32 threads) and the in-RAM tile builder are the known obstacles.
4. Wasted area from circle nesting grows with hierarchy depth.
5. Viewer performance without a GPU is poor as measured.

**Effort to productionise:** roughly 6–10 person-weeks (streaming tile writer and quantised payload 2; layout parallelism and power-diagram trial 2–3; edge model and per-node adjacency 2–3; viewer hardening 1–2).
