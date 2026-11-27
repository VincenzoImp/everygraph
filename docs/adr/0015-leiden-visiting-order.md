# ADR 0015 — Leiden with block-permuted visiting order and a connectivity guarantee

- **Status:** accepted, partly validated
- **Decision:** D15 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Community detection is the heart of the map. Fast parallel implementations (GVE, PLM)
visit vertices in id order; Louvain can return disconnected communities; one Leiden
iteration is fast but below the best references.

## Decision

Leiden, visiting vertices in a block-permuted order (blocks of 64 ids), with an explicit
step that splits disconnected communities. One iteration is the fast default; three is
"quality".

## Evidence

All idle, 16 threads, async mode unless noted (FINDINGS § B2 and "Idle-machine re-runs";
`lab/b2-leiden/results/summary.txt`, `lab/b2-leiden/results/quality/`,
`lab/b2-leiden/results/scaling/`, `lab/b2-leiden/results/refs/`,
`lab/a-baselines/results/baselines.jsonl`):
- **Order vs algorithm**: id order costs lj 1.95 and orkut 2.14 modularity points against
  block-permuted order at one iteration (0.7302 vs 0.7497; 0.6475 vs 0.6689), and 3.45 and
  4.47 points against three iterations. Louvain vs Leiden at the same order differ by at
  most 0.37 points (lj 0.7483 vs 0.7497; orkut 0.6726 vs 0.6689). Order matters more, as
  DESIGN says. Like for like on the idle machine it is 2.0–2.1 points at one iteration;
  shared-machine runs on 6 dev cores gave lj 4.2 and orkut 2.6 points.
- **Price of the order**: block-permuted takes 1.38× (lj 0.76 vs 0.55 s) and 1.63×
  (orkut 1.43 vs 0.88 s) the id-order time. FINDINGS' "1.5–2×" is from shared runs; the idle figure was added on 2026-11-25.
- **Iterations**: three iterations reach lj 0.7647 and orkut 0.6922. Best references:
  lj 0.7704 (NetworKit ParallelLeiden at its defaults, 3 iterations with randomized
  order, `refs/lj_nk_leiden_rand.json`) and
  0.7695 (igraph Leiden, 2 iterations, `baselines.jsonl`); orkut 0.6924 (igraph) and 0.6911
  (NetworKit randomized). Three iterations are 0.62–0.74% below on lj and between 0.03%
  below and 0.16% above on orkut. One iteration is 2.7% (lj) and 3.2–3.4% (orkut) below;
  FINDINGS' "0.7–2.7%" is from shared runs; the idle figure was added on 2026-11-25.
- **Connectivity**: Leiden left 0 disconnected communities in every run; Louvain left 85
  (lj) and 18 (orkut). The split-last step fired: +27 communities on lj, +476 on
  Friendster (FINDINGS § B2; shared).
- **Narrowness**: two undirected, unweighted social graphs. No web, road, bipartite,
  weighted or directed graph (DESIGN § 5, item 7).
- *Added 2026-11-25 (#16, FINDINGS § M1-02):* a seeded relabelling of ids at ingest with
  storage-order visits (L1) was measured promising on Friendster (async, 1 iteration, 16
  threads, idle: Q 0.611 against 0.605, and level-0 local moving at its in-RAM cost under
  8 and 4 GiB; `bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl`). #146
  (`plan/issues/m1-14-relabel-at-ingest.md`) rejected it: a random relabelling
  costs 5–31% of storage and slows every kernel, and Leiden's quality does not need it.
  The decision is unchanged.

## Consequences

- Production Leiden (#57) is built with these choices. #259 confirms or changes the
  default of one iteration with quality and time at 1, 2, 3 iterations and at
  convergence on six graph families (`design/DECISIONS.md`, 2026-11-25).
- The connectivity step is part of every mode, deterministic and fast.

## What would change our mind

- On #24's graph families, block-permuted order does not beat id order by at least one
  modularity point, or costs more than 2× the time.
- Three iterations fall more than 1% below the best reference on any of the six families
  (the target in DESIGN § 6).

## Closed by

#24 (graph families beyond social networks), #259 (default number of iterations, measured on production Leiden).
