# ADR 0014 — Deterministic by default, across threads, platforms, budgets and regimes

- **Status:** accepted, partly validated
- **Decision:** D14 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

"See it and trust it" (DESIGN § 2, promise 3) needs a result that does not depend on how
it was computed. D14 first promised identity across thread counts and platforms. On
2026-11-25 it was widened (`design/DECISIONS.md`, row "What 'repeatable' promises")
because the planner may run the same call in memory or out of core, and a view as a mask
or materialized: if those choices changed the answer, the memory budget would become a
question of answer, not of time.

## Decision

A deterministic run, the default, returns the same result for any number of threads, on
any platform, **for any memory budget and whichever way the planner executes it** (in
memory or out of core; a view as a mask or materialized). The planner never substitutes a
lower-precision variant by itself: it refuses and offers it. Not covered: results cut
short by `timeout`, and `deterministic=False` (the opt-in `fast` mode for community
detection). If a kernel cannot meet the promise, it is narrowed by a new ADR, not
silently.

## Evidence

- **Threads** (FINDINGS § B2 and "Idle-machine re-runs"; idle; lj and orkut;
  `lab/b2-leiden/results/summary.txt`, `lab/b2-leiden/results/scaling/`): deterministic
  Leiden and Louvain give the same hash of all level maps at 1, 2, 4, 8, 16 and 32
  threads; three-iteration Leiden the same hash at 16 and 32 (`lab/b2-leiden/results/quality/`).
- **Platforms** (FINDINGS § G; shared; lj): PageRank and components agree across native
  x86-64, wasm32, wasm64, Node and Chromium at 1–32 threads, compared by PageRank sum and
  maximum and component count, not by a hash of the whole output (`lab/g-wasm/results/`).
  Leiden ran on native x86-64 only; nothing ran on ARM.
- **Map**: layout positions and the PMTiles file are byte-identical across thread counts
  on lj and orkut, after one thread-dependent float merge was fixed (FINDINGS § E; shared;
  `lab/e-map/results/determinism_lj.json`, `lab/e-map/results/determinism_tiles_orkut.json`).
- **Budgets and regimes: not measured for any kernel.** The only evidence of identity
  across regimes is ingest: text → CSR was byte-identical to the reference in RAM, out of
  core and from gzip (FINDINGS § B1; `lab/b1-core/results/bench/validation.txt`).
- **Weights**: bit-identity needs exact totals — fine for unweighted and integer weights;
  f32 weights need fixed-point totals, designed but not implemented (FINDINGS § B2).
- **Cost** (idle; deterministic ÷ asynchronous wall time at the same thread count;
  `lab/b2-leiden/results/summary.txt`): Leiden at 16 threads 3.02× on lj and 2.43× on
  orkut (2.51× and 1.96× at 8; 3.20× and 3.04× at 32); Louvain at 16 threads 2.00× and
  1.37×. Deterministic Leiden stops scaling at about 8 threads (lj 2.56 s at 8, 2.30 s at
  16, 2.51 s at 32). Modularity: lj 0.7480 vs 0.7497 async, orkut 0.6760 vs 0.6689. Two
  social graphs of 35 M and 117 M edges; never run at 10^9 edges.

## Tests that hold it

- #54: hashes at 1, 2, 3, 8 threads and oversubscribed, native vs wasm32; the same hashes
  under each forced regime, two memory budgets, and a view as mask and materialized.
- #47: f64 PageRank, identical bytes at 1, 4, 16 threads, in each regime, at budgets of 2,
  8, 16 GiB and unlimited, and on a view either way; f32 only on request.
- #48: component labels identical across threads, Afforest vs streaming union-find,
  budgets and views.
- #93: PageRank, components and communities identical on a view as mask or materialized,
  and after `save()`.
- #45: the result hash is the same in every regime the planner can choose.

## Consequences

- f64 state is the default wherever the result is a float; f32 is offered, never chosen.
- The case that could narrow the promise is **out-of-core Leiden** (#60, acceptance:
  "deterministic mode works out of core with the same hash as in RAM"). Streaming
  aggregation combines (community, community, weight) triples in bucket order; with
  non-integer weights the totals are exact only with the fixed-point scheme not yet built.
- The price for community detection is above the target of #18 (≤ 1.5× async at 16
  threads); it is stated with the numbers above wherever the default is explained.

## What would change our mind

- #60 cannot reach the in-RAM hash out of core within its time and memory targets: the
  promise is narrowed, by ADR, for community detection beyond RAM.
- #18 leaves deterministic Leiden above 2× async at 16 threads on lj, orkut and
  Friendster: the default mode for community detection is reconsidered by ADR.
- #54 finds float results that cannot be made identical across x86-64 and ARM without
  unacceptable cost.

## Closed by

#18 (cost), #54 (the tests across threads, platforms, budgets and regimes), #60 (the case
that would narrow it).
