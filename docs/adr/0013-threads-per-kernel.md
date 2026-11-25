# ADR 0013 — Thread count is chosen per kernel

- **Status:** accepted, partly validated
- **Decision:** D13 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The machine has 16 cores and 32 hardware threads. Memory-bound kernels may gain nothing
from SMT siblings or lose to contention; compute-bound kernels may gain. Using all threads
everywhere is the naive default.

## Decision

The planner chooses the thread count per kernel: physical cores for memory-bound scans,
all hardware threads for compute-bound kernels.

## Evidence

From 16 to 32 threads (FINDINGS "Idle-machine re-runs"; idle; uncompressed arrays;
`lab/b1-core/results/bench/kernels.jsonl` for orkut kernels,
`lab/b2-leiden/results/summary.txt` and `lab/b2-leiden/results/scaling/` for communities):
- orkut, no gain or a loss: PageRank f32 0.37 → 0.38 s; components 13 → 15 ms; BFS
  22 → 22 ms; k-core 0.30 → 0.37 s.
- orkut, a gain: triangles 2.51 → 1.46 s; Louvain async 0.88 → 0.65 s; Leiden async
  1.43 → 1.24 s.
- lj: Leiden async 0.76 → 0.78 s; Louvain async 0.42 → 0.38 s (small gain); and on both
  graphs every deterministic run is slower at 32 (Leiden det lj 2.30 → 2.51, orkut 3.48 → 3.77 s;
  Louvain det lj 0.85 → 0.99 s). DESIGN quoted only orkut Louvain until on 2026-11-25; the gain for
  community detection depends on the graph and the mode.
- Friendster: PageRank looked flat from 8 threads up (FINDINGS § B1), but only on a loaded
  machine; there is no idle scaling curve, and the memory-bandwidth explanation is not
  confirmed with counters.

Evidence is two graphs for communities, one for the other kernels, one machine.

## Consequences

- The planner (#45) carries a per-kernel thread policy. For community detection the policy
  depends on the mode (deterministic runs lost from SMT on both graphs) and is revisited
  with #18.
- Benchmarks state the thread count used for every number (CLAUDE.md § 3).

## What would change our mind

- #22's idle Friendster curve shows memory-bound kernels gaining more than 10% from 16 to
  32 threads, or cache blocking changes which kernels are memory-bound.
- #24 shows community detection on other graph families losing from SMT in async mode.

## Closed by

#22 (memory-bandwidth ceiling, idle Friendster scaling); #18 for deterministic community
detection.
