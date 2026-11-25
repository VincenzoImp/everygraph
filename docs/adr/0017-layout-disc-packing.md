# ADR 0017 — Layout by hierarchy-constrained disc packing

- **Status:** accepted, partly validated
- **Decision:** D17 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The map needs positions for every node at every level, computed in minutes for 10^9
edges, identical across runs and thread counts (ADR 0014), and readable: nearby things
should be related.

## Decision

The layout is a hierarchy-constrained disc packing (each community a disc with area
proportional to its node count, a small force solve per parent), deterministic.

## Evidence

From the map experiment (FINDINGS § E; shared machine; `lab/e-map/results/`):
- **Cost**: lj (4.0 M nodes) 3.3 s at 32 threads, 23.1 s at 1 thread (7×); orkut 8.1 s and
  51.3 s; peak RSS 0.88 and 2.1 GB (`lab/e-map/results/layout_lj_t32.json`,
  `lab/e-map/results/layout_lj_t1.json`, `lab/e-map/results/layout_orkut_t32.json`).
- **Edge length** (median edge length ÷ median random-pair distance; lower is better):
  0.014 for disc packing on lj against 0.244 for a FastRP-like embedding with PCA, a
  featureless blob (`lab/e-map/results/eval_lj_hcl.json`,
  `lab/e-map/results/eval_lj_fastrp.json`). On orkut disc packing gives 0.074
  (`lab/e-map/results/eval_orkut_hcl.json`), which DESIGN does not quote.
- **Determinism**: positions byte-identical across thread counts on lj and orkut after one
  fix (`lab/e-map/results/determinism_orkut.json`).
- **Wasted area**: node discs cover "about 1–3%" of the map (`lab/e-map/REPORT.md`, an
  estimate). DESIGN's earlier "wastes 97% of the area" was not traced to a result file and
  was replaced on 2026-11-25 by the estimate, labelled as such.
- "Readable" is the experiment's visual judgment, not a metric.
- **At 10^9 edges**: 2–4 minutes and about 20 GB of RAM — an extrapolation, not measured
  (FINDINGS § E). *Replaced 2026-11-25 by a measurement* (#15, FINDINGS § M1-01; idle;
  Friendster, 1.8 B edges, 32 threads, capped Louvain hierarchy, 11.35 M tree nodes;
  `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`):
  69.6 s with 17.7 GiB anonymous, graph in the heap, 56 GiB limit (rows 22–23). Under
  16 GiB it is killed with the graph in the heap and completes with the graph
  memory-mapped, 405.1 s and 3.7 GiB anonymous, re-reading 733 GB with default readahead
  (rows 18–19). On the
  plain Louvain hierarchy (a parent with 18.7 M children) it is killed even with 56 GiB
  (rows 20–21): memory for a parent with millions of children is not bounded (#67).

## Consequences

- #67 builds the layout; it needs a good hierarchy first (ADR 0016).
- The "pull towards external neighbours" term showed no measurable gain (mean ratio 0.226
  with it, 0.223 without; `lab/e-map/results/eval_lj_hcl_noext.json`) and is redesigned or
  dropped in #21.
- The 1.8 B-edge layout is measured end to end by #15 before any claim about it.

## What would change our mind

- #21 finds a layout with the same cost and determinism that occupies at least 10× more of
  the map, or improves the edge-length ratio on orkut below 0.05; disc packing is then
  replaced in #67.

## Closed by

#21 (layout area and meaningful distances).
