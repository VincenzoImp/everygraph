# ADR 0018 — Data tiles in PMTiles v3 and a dependency-free WebGL2 viewer

- **Status:** accepted, partly validated
- **Decision:** D18 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The map must open from a static URL, show exact numbers at every level, and stay small
over the network. Pre-rendered pixels fix the style and lose the numbers; shipping the
whole graph does not scale.

## Decision

The map is a pyramid of data tiles (positions, sizes, aggregates), not pixels, stored in
one PMTiles v3 file and drawn by a dependency-free WebGL2 viewer, in a static mode (file
only) and a live mode (engine behind it).

## Evidence

From the map experiment (FINDINGS § E and R5; shared machine; sizes are unaffected by
load; `lab/e-map/results/tiles_lj_t32.json`, `lab/e-map/results/tiles_orkut_t32.json`,
`lab/e-map/results/viewer_lj.json`, `lab/e-map/results/viewer_orkut.json`,
`lab/e-map/results/viewer_lj_software.json`):
- **Format**: a PMTiles v3 file with a custom payload opened in the official Python
  reader.
- **Network** (lj / orkut): first view 152 / 143 KB in 18 requests; tile gzip median
  4.2 / 5.9 KB, maximum 20.5 / 21.6 KB; cold search 2 requests (47 / 44 KiB); a
  zoom-to-node session 5.0 / 3.9 MB in 546 / 454 requests.
- **Build**: 8.5 / 15.0 s at 32 threads; 851 / 460 MB in one file.
- **Exactness**: 1,403,208 lj community aggregates matched the community experiment with 0
  mismatches.
- **Drawing**: 16.67 ms frames on the RTX 4090 (vsync-capped); with software rendering
  (SwiftShader) 29–136 ms mean and 567 ms worst — not interactive without a GPU.
- **What does not work yet**: edges become fog inside giant communities; 47% of edge weight
  undrawn at the deepest zoom on lj, 90% on orkut (DESIGN § 5, item 3).
- **Not measured**: live mode; Firefox and Safari; integrated GPUs; any graph above
  117 M edges. At 10^9 edges, 9–13 GB of tiles is an extrapolation (FINDINGS § E).
- **Friendster, measured 2026-11-25** (#15, FINDINGS § M1-01; idle; 1.8 B edges, 32
  threads, capped Louvain hierarchy;
  `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`), replacing that extrapolation: 847,925 tiles,
  zmax 11, a 6.84 GB file (5.79 GB of gzip tiles, 1.05 GB search index), tile median
  6.8 KB and maximum 32.8 KB gzip, built in 244.5 s with 29.1 GiB anonymous under 56 GiB
  (row 29). Under 16 GiB the builder, which keeps every tile and each zoom level's
  working state in RAM (13.9 GiB anonymous), was cut by the 60-minute timeout before the
  last zoom level (row 27; #69). Viewer on the RTX 4090
  (row 31): first view 163,184 B in 18 requests, a zoom-to-node session 5,624,158 B in 577
  requests, cold search 289,056 B in 2 requests; frames vsync-capped at 16.67 ms. 7.4% of
  edge weight is drawn at the deepest zoom.

## Consequences

- #68 specifies the tile format, #69 builds tiles, #71 the viewer, #74 live mode.
- The edge strategy is a separate decision fed by #20.
- The first-view target of DESIGN § 6 (< 250 KB for a 1 B-edge graph) is checked on the
  Friendster tracer (#15), not assumed from lj.

## What would change our mind

- #26 shows integrated GPUs cannot hold 30 fps with data tiles under any per-frame cap,
  so an overview of pre-rendered pixels becomes necessary.
- #15 measures a first view above 250 KB on Friendster. *(2026-11-25: it measured
  163,184 B; this did not fire.)*

## Closed by

#77 (the viewer on an integrated GPU, Firefox and Safari; WebGL 2 is the minimum, `design/DECISIONS.md`), #69 (the tile pyramid of a 1.8 B-edge graph under a budget).
