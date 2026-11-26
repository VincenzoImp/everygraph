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
  Friendster at 32 threads peaked at 23,648,132 KiB of anonymous memory (22.6 GiB; DESIGN,
  FINDINGS and README wrote "23.6 GB" until on 2026-11-25) and does not fit 16 GiB. Louvain under 16 GiB finished
  with 3.8 GiB anonymous but Q 0.4647 against Leiden's 0.6033. On orkut with a limit near
  the edge-array size, Leiden ran 5.7× slower (23.8 vs 4.2 s, 6 threads); below it,
  Louvain ran 51× slower (163.9 vs 3.2 s) or was killed.
- **Measured end to end, 2026-11-25** (#15, FINDINGS § M1-01; idle; Friendster, lab
  binaries, 32 threads, f32 quotient weights, uncompressed CSR memory-mapped;
  `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`):
  - Under 16 GiB, Leiden and fan-out-capped Louvain are killed in pass-0 aggregation, after
    the local moves, while re-reading the mapped graph (1.80 TB and 2.72–2.74 TB read;
    rows 9, 11–12). The level-1 graph is the term that does not fit, as above.
  - Plain Louvain under 16 GiB finished in 115.1 s with 4.2 GiB anonymous and Q 0.6038
    (row 15): the Q 0.4647 above was not reproduced (one run each, not diagnosed, #17).
  - With hierarchy output the prototype keeps every level's quotient graph in memory: the
    capped Leiden was killed at 56 GiB (about 51.5 GiB of quotient graphs by arithmetic;
    not tested without hierarchy output). The capped Louvain completed with a 42.9 GiB
    anonymous peak (rows 13–14), 28.3 GiB before writing the hierarchy; the rest is most
    likely the writer's transient f64 copy of the weights (inference, review of #15's lab
    PR).
  - The map stages follow the regime rule only when their graph is not in the heap: the
    layout fits 16 GiB with the graph memory-mapped (3.7 GiB anonymous) at 5.8× its time
    with 56 GiB, re-reading 733 GB with default readahead (rows 19, 22–23); the tile
    builder, which keeps every tile and each zoom level's working state in RAM, did not
    finish in 60 minutes (row 27).
  None of this is #16's experiment, so "What would change our mind" has not fired.

So the rule is measured for PageRank and components on one graph (Friendster). It is not
valid for community detection, whose memory is set by an intermediate graph.

## Result of #16 (2026-11-25)

FINDINGS § M1-02; idle; Friendster, Leiden, 1 iteration, 16 threads, f64 weights, level 0
in the D4 compressed store; lab code `lab/m1-02-ooc-leiden`;
`bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl` and
`…20261125T174959Z-b61036e59740.jsonl` (official, 0 disturbed).

- Community detection fits the regimes once its intermediate graph is treated like the
  input: aggregation streams in node order into range-partitioned buckets held in RAM
  while the accountant allows and spilled to files otherwise, and every quotient graph is
  written as a compressed store and memory-mapped. Every large allocation is reserved in
  an accountant stub first; the anonymous peak stayed under the hard limit in all 31 rows.
- Against the same algorithm in RAM (74.8 s, nothing spilled), async: 16 GiB 87.4 s
  (1.17×), 8 GiB 175.3 s (2.34×), 5 GiB 2.88×, 4 GiB 3.49×; 0 disconnected communities
  everywhere; Q 0.598–0.607 in RAM and out of core alike, with one of the two 16 GiB
  repetitions at 0.5981. Deterministic: the same partition in RAM and under 16 GiB (hash
  `2a12e5882874c38d`, Q 0.6055). Map-shaped Leiden completes under 16 GiB (164.1 s).
- The cost below 16 GiB is level-0 local moving re-reading the store 6.0–12.8× per sweep
  in block-permuted order, plus 1.6–2.0× on level-0 aggregation from bucket files.
- "What would change our mind" (first bullet) did not fire: under 16 GiB the run
  completes with 0 disconnected communities within 2.5× (1.17×), and Q ≥ 0.60 holds in
  the deterministic mode and for the async median, though not for every async repetition.
  The decision stands. Its status stays *accepted, partly validated*: one graph, one
  iteration, the deterministic mode not measured below 16 GiB, and a lab prototype, not
  the product's planner and accountant. Those close in #60 (production out-of-core
  Leiden); the iterative-scan half still closes in #46.

## Consequences

- The planner (#45) sizes the O(n) state per kernel and refuses with the minimum budget
  when it does not fit; the semi-external source is #46; the no-OOM matrix is #53.
- For community detection the planner must also estimate the level-1 graph (22–44% of
  input entries in the lab) — part of #60 — and, for the map hierarchy, every level's
  quotient graph unless they are streamed to disk (#62; added 2026-11-25 from #15).
- The documentation says plainly that out of core costs about one read of the graph per
  pass: small for PageRank, 10× for components on Friendster.

- Regime 2 for random access whose working set exceeds the budget: amended by
  [ADR 0029](0029-random-access-under-a-budget.md), accepted 2026-11-26 (an explicit
  budgeted cache over `ByteSource`; per-phase `MADV_RANDOM` until it exists), from #172.

## What would change our mind

- #16 cannot run Leiden on Friendster under 16 GiB with Q ≥ 0.60, no disconnected
  communities, and ≤ 2.5× the in-RAM time: the product claim narrows to the smallest
  budget #16 reaches, by ADR.
- #46 shows Friendster PageRank under 2 GB beyond 1.35× of in-RAM (its acceptance).

## Closed by

#60 (production out-of-core Leiden) for community detection; #46 for iterative scans; #178
(the block cache, ADR 0029) for random access under a budget.
#16 reported on 2026-11-25 (above).
