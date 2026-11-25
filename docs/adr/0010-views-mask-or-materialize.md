# ADR 0010 — Views: mask or materialize, chosen by the planner

- **Status:** open — needs experiment
- **Decision:** D10 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Filtered views ("edges above 10 ETH", "only 2024", "only this community") must work with
every kernel. A view can be a mask over the full graph or a materialized filtered
adjacency. DESIGN states a rule: bitset mask above ~25% selectivity, materialized below.

## Decision

Kept: both representations exist and the planner chooses between them. **Not ratified:**
the ~25% selectivity threshold. The evidence below does not support it, so the planner
rule waits for a measurement.

## Evidence

From the properties and views experiment (FINDINGS § F1; shared machine; orkut; one
PageRank iteration and one BFS; time relative to the unfiltered graph;
`lab/f1-query/results/props_orkut_t32.json`, `lab/f1-query/results/props_orkut_t4.json`):

| selectivity | entry-bitset mask, PR iter (32 / 4 threads) | materialized, PR iter (32 / 4) | materialized build (32 / 4) |
|---|---|---|---|
| 1% | 1.03× / 1.02× | 0.090× / 0.087× | 47 ms / 293 ms |
| 10% | 0.98× / 2.09× | 0.17× / 0.091× | 69 ms / 142 ms |
| 50% | 1.42× / 1.69× | 0.81× / 0.36× | 135 ms / 595 ms |
| 90% | 0.91× / 0.98× | 0.69× / 0.75× | 111 ms / 242 ms |

- A plain bitset mask never clearly beats the unfiltered scan (0.91–2.09×), as FINDINGS
  says. A Roaring mask did at 1% (0.59× at 32 threads).
- DESIGN's "PageRank at 0.09× for 1–10% selectivity" holds at 4 threads; at 32 threads it
  is 0.09× at 1% and 0.17× at 10%.
- **Where the rule is not supported:** there is no measurement between 10% and 50%, and
  at every selectivity measured the materialized view was faster per PageRank iteration
  than the mask. Over 20 iterations it also wins with its build time included, at 50%
  (32 threads: mask 20 × 77 ms = 1.54 s; materialized 135 ms + 20 × 44 ms = 1.01 s) and at
  90%. The mask wins only for few-pass work (BFS at 50%, 32 threads: 0.23 s masked vs
  0.28 s build plus BFS) and on memory (the 50% materialized view takes 493 MB, the
  bitset 29 MB). The choice therefore depends on the number of passes and on the memory
  budget, not on selectivity alone. FINDINGS' "~20–30%" is not traced to a measurement.

## Consequences

- #93 builds both representations; its planner rule is derived from a crossover
  measurement, not from the ~25% figure.
- The determinism promise covers views either way (ADR 0014): a kernel returns the same
  result on a mask or a materialized view.

## What would change our mind

This ADR moves to `accepted` when a measurement on at least two graphs (orkut and
twitter-sym), idle machine, 16 threads, at 1, 5, 10, 25, 50 and 90% selectivity, gives
the crossover for PageRank (20 iterations), BFS and components, including build time and
memory, and the planner rule is written from it.

## Closed by

#93 (filtered views), with the planner rule in #45. #93's scope does not yet include the
measurement above; adding it is an amendment to #93.
