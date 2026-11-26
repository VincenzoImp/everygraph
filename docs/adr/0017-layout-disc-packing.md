# ADR 0017 — Layout by hierarchy-constrained disc packing

- **Status:** accepted, partly validated
- **Decision:** D17 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25
- **Amended:** 2026-11-26, with the result of #175 (comparison with published layouts,
  narrower claim, stress solve per parent, new re-decision trigger)

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
- #15 measured the 1.8 B-edge layout end to end (above); no claim about it goes beyond
  that run until #67 bounds its memory.

## What would change our mind

- #21 finds a layout with the same cost and determinism that occupies at least 10× more of
  the map; disc packing is then replaced in #67.
- A layout with the same cost and determinism beats ours on the four measures of the
  amendment below (NP@10 per quotient level, ground-truth 10-NN agreement, Spearman, length
  of edges between top regions), each measured against DRGraph. The earlier trigger
  "edge-length ratio on orkut below 0.05" is withdrawn: no layout tested reaches it.

## Amendment (2026-11-26, #175): compared with published layouts

The decision stays: hierarchy-constrained disc packing. Four things change.

**1. The evidence now includes published layouts** (FINDINGS § M1-18;
`lab/m1-18-layout-baseline/results/README.md`, `lab/m1-18-layout-baseline/results/table.md`).
Same hierarchy (`data/hier/<g>`), lj and orkut, 8 threads unless stated, hard memory
limits. The baselines are DRGraph@4f3fda2 with its README options, and ForceAtlas2 in
cuGraph 26.10 on the GPU (a reference only; the product needs no GPU). The metrics are
deterministic (fixed seeds, one script for every layout). All times and memory figures are
exploratory: the machine was shared, load average 14–62.

- **Ahead on local and community structure.** NP₁ is 16–50× DRGraph's and FA2's (lj 0.247
  against 0.015; orkut 0.102 against 0.002). NP@10 per quotient level is 4–50× DRGraph's
  on lj L0–L4 and 2–90× on orkut L0–L3. Agreement of the 10 nearest nodes with SNAP's
  ground-truth communities is 0.79 against 0.55 (lj) and 0.52 against 0.04 (orkut).
- **Behind on global structure.** Spearman's ρ between layout distance and hop distance is
  0.03 on both graphs, the same as a random layout; DRGraph gives 0.31 (lj) and 0.55
  (orkut). 21% (lj) and 24% (orkut) of edges cross between top regions. In our layout
  their median length is 0.98 and 1.27 of a random pair; in DRGraph 0.23 and 0.16. On
  orkut even the median edge ratio is behind (0.088 against 0.070).
- **Not completed under 24 GiB on lj:** Graphviz sfdp 14.1.2 and OGDF FM³
  (foxglove-202510) were killed before they produced a layout. BatchLayout@d2e260d with its
  default greedy start was killed at 16 GiB. With a random start it ran 4,795 s and stayed
  close to random (median edge ratio 0.80). This is not a fair picture of the method.
- **Cost, exploratory:** ours 4–12 s and at most 1.2 GB; DRGraph 700 s and 7.0 GB (lj),
  919 s and 18.4 GB (orkut).

**2. The claim is narrower.** The layout places nodes near their neighbours and
communities near their strongest neighbours. Distance between large regions carries
little meaning. The map's text and legend must not suggest otherwise. Do not judge the
layout on the mean or p90 edge ratio alone.

**3. The per-parent solve becomes stress on the quotient graph** (`--solve stress`,
`lab/e-map/gmap/src/stress.rs`). It is deterministic: the output is byte-identical at 3
and 8 threads on lj. It costs about 1.8× the solve CPU (interleaved A/B on lj,
`lab/m1-18-layout-baseline/results/ab_lj.jsonl`). It raises NP₁ and NP@10 on both graphs.
On orkut it brings Spearman from 0.03 to 0.33 and the top crossing edges from 1.27 to 0.88
of a random pair. On lj, where each top region links to 44 of the other 70, it gives no
global gain. #67 adopts it, once #21 has settled the area question.

**4. The re-decision trigger changes** (see "What would change our mind"). It is now four
measures, each against DRGraph: NP@10 per quotient level, ground-truth 10-NN agreement,
Spearman, and the length of edges between top regions.

**Open.** The remaining global gap is a method item in #21. Hypothesis, not measured: it
comes from where members with outside links sit inside each region. On orkut DRGraph's
top regions are as compact as ours (0.14 against 0.12 of a random pair), yet its edges
between them are much shorter. A rigid rotation per sibling group did not help.

## Closed by

#21 (layout area and meaningful distances).
