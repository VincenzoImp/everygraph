# ADR 0011 — Three execution regimes, chosen by the size of the O(n) state

- **Status:** accepted, partly validated
- **Decision:** D11 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The product promises never to run out of memory under a user's budget (DESIGN § 2,
promise 2). The engine must pick, before it starts, how to run a kernel on a graph that
may not fit in RAM.

## Decision

Three regimes: mmap warm (the only "in-RAM" mode), mmap under memory pressure, and
semi-external streams with a fixed RSS. The planner chooses by the size of the kernel's
O(n) state, not by the size of the graph. A memory accountant backs the no-OOM guarantee.

## Evidence

- **mmap warm = heap** (FINDINGS § B1 probes; idle; orkut, 32 threads,
  `lab/b1-core/results/bench/kernels.jsonl`, label `probe`): PageRank f32 0.380 s mmap vs
  0.375 s heap; f64 0.435 vs 0.468 s.
- **Iterative scan out of core** (FINDINGS "Idle-machine re-runs"; idle; Friendster
  1.8 B edges, PageRank f32, 32 threads; `lab/b1-core/results/bench/kernels.jsonl`,
  `lab/b1-core/results/bench/semiext.jsonl`): 3.37 s/iteration in RAM (mmap) vs
  semi-external 3.71 (no limit), 3.69 (8 GB limit), 3.64 s (2 GB limit), RSS 1.35–1.47 GB:
  +8–10%. PageRank's in-RAM iteration happens to cost about one sequential read of the
  graph from this NVMe, which is why the penalty is small.
- **Single-pass kernel out of core** (same files): Friendster components 0.34 s in RAM vs
  3.55 s semi-external under 8 GB — 10×, I/O-bound.
- **The only OOM** (FINDINGS § B1; shared): semi-external components under a 1 GB limit
  was killed (exit 137) — its O(n) state did not fit (`lab/b1-core/results/ooc_friendster.jsonl`,
  label `lim1-wcc-semiext`).
- **Community detection does not follow the rule** (FINDINGS § B2; shared;
  `lab/b2-leiden/results/big/friendster_leiden_inram34g.json`,
  `lab/b2-leiden/results/big/friendster_louvain_semiext16g.json`,
  `lab/b2-leiden/results/semiext/`): Leiden's O(n) state is small (21–53 B/node) but its
  dominant term is the level-1 quotient graph (22–44% of input entries). Leiden on
  Friendster at 32 threads peaked at 23,648,132 KiB of anonymous memory (22.6 GiB; DESIGN
  and FINDINGS write "23.6 GB") and does not fit 16 GiB. Louvain under 16 GiB finished
  with 3.8 GiB anonymous but Q 0.4647 against Leiden's 0.6033. On orkut with a limit near
  the edge-array size, Leiden ran 5.7× slower (23.8 vs 4.2 s, 6 threads); below it,
  Louvain ran 51× slower (163.9 vs 3.2 s) or was killed.

So the rule is measured for PageRank and components on one graph (Friendster). It is not
valid for community detection, whose memory is set by an intermediate graph.

## Consequences

- The planner (#45) sizes the O(n) state per kernel and refuses with the minimum budget
  when it does not fit; the semi-external source is #46; the no-OOM matrix is #53.
- For community detection the planner must also estimate the level-1 graph (22–44% of
  input entries in the lab) — part of #60.
- The documentation says plainly that out of core costs about one read of the graph per
  pass: small for PageRank, 10× for components on Friendster.

## What would change our mind

- #16 cannot run Leiden on Friendster under 16 GiB with Q ≥ 0.60, no disconnected
  communities, and ≤ 2.5× the in-RAM time: the product claim narrows to the smallest
  budget #16 reaches, by ADR.
- #46 shows Friendster PageRank under 2 GB beyond 1.35× of in-RAM (its acceptance).

## Closed by

#16 (out-of-core Leiden aggregation) for community detection; #46 and #60 productionize.
