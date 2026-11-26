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

"The same input" is the same stored graph, **its stored order included**: the ids and
the order of the lists. The same graph with its ids renumbered (`compact --order`,
m2-24) is another input, and its communities may differ. A view is the same input
whichever way it runs: a mask over its graph or materialized, its nodes numbered in
view order. Nodes without edges change no community. A map rebuilt with the previous
map's super-level decision (ADR 0016) depends on that decision: it is an input of the
build, so a rebuilt map depends on its history.

## Reproducibility

Each property is guaranteed (a test holds it), shown in the lab only, or not promised.
The tests named are in `crates/everygraph-determinism/tests/` unless a path is given.

| Property | Status | Held by |
|---|---|---|
| Same seed, run again | guaranteed | `suite.rs`: the grid of every kernel of `everygraph run` and of Leiden (default, quality, randomized) ends with its first configuration again; the build test builds its first configuration twice |
| Same thread count | guaranteed | as the row above |
| Any thread count | guaranteed | `suite.rs` `the_build_gives_the_same_bytes_everywhere`, `kernels_give_the_same_bits_everywhere`, `leiden_gives_the_same_bits_everywhere`: 1, 2, 3, 8 and 32 threads (32 on 4 cores in CI); lab: Leiden and Louvain on lj and orkut at 1–32 threads |
| Scheduling order: tasks, chunks, decode batches, the order threads finish | guaranteed | the same tests, at tasks of 64 and 65,536 entries; `everygraph-kernels/tests/framework.rs` `every_source_thread_count_chunk_and_batch_give_the_same_bits`; order probes `suite.rs` `order_probes_reveal_the_specified_summation_tree`. No float reduces over the framework's blocks: there is no `Reduce` for `f32` or `f64` |
| In core vs out of core | build: guaranteed; kernels and Leiden: shown in the lab | build: `the_build_gives_the_same_bytes_everywhere` (in RAM and spilled, and at the smallest budget that builds); every source, the streamed one included: the suite's grid; budgets: `everygraph-community/tests/leiden.rs` `results_do_not_depend_on_the_budget`; out-of-core Leiden on Friendster gave the in-RAM hash (`lab/m1-02-ooc-leiden`); the product test is #60's |
| Across CPUs: x86-64 with any `target-cpu`, aarch64, i686 with SSE2, wasm32 | primitives: guaranteed; kernels and Leiden: shown in the lab | `everygraph-core/tests/numerics_golden.rs` (the pinned math, the exact accumulator, the counter-based generator) on x86-64, i686 and wasm32 in CI; PageRank and components across native and wasm by sum and maximum (FINDINGS § G); Leiden ran on x86-64 only. A full hash across targets is #54's R15e |
| CPU vs GPU | not promised | no kernel runs on a GPU |
| Bitwise | guaranteed | every test above compares bits: hashes of the output columns, floats by their bits |
| Partition up to renumbering | guaranteed, as bits | the membership is numbered by each community's smallest node, so the same partition has the same bits |
| Same stored order vs a permuted one | not promised | `compact` without `--order` keeps the communities: `leiden_views.rs` `compact_keeps_the_communities`. With the ids renumbered by a locality order or at random, 4 of 5 orders changed the communities of a planted graph of 10,000 nodes, by 7 to 131 nodes (#54's probe; PR #NNN) |
| View as a mask vs materialized | guaranteed for Leiden | `leiden_views.rs` `a_view_as_a_mask_and_materialized_give_the_same_communities`, default, quality, randomized and prolonged; other kernels: #93 builds views, with this test |
| Nodes without edges | guaranteed for Leiden | `leiden_views.rs` `nodes_without_edges_change_neither_the_sub_rounds_nor_the_communities`: the sub-round count counts nodes with edges |
| A map after a rebuild | not promised: depends on the previous map | ADR 0016: the super level keeps the previous map's choice while the partition needs 56 to 72 communities to cover 90% of its nodes; a first map depends on the graph and the seed only |
| `fast` mode, `timeout` | not promised | — |

How Leiden keeps the view rows (`everygraph-community/src/level.rs`, "Deterministic
mode"): every draw is counted by a node's place among the nodes with edges, not by its
id; the sub-round count is a function of how many they are; ties go by the draws of the
candidate communities; and a community starts named by its smallest node. Labels are
then compared only by their order, which a mask and a materialized view share.

**The price in quality.** QUALITY_PLACEHOLDER

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

- #54 (`crates/everygraph-determinism`, the table above): hashes at 1, 2, 3, 8 threads
  and oversubscribed, two chunkings, four sources and a repeated run; Leiden on a view
  as a mask and materialized. Left: native vs wasm32 and aarch64 by a full hash, the
  forced regimes and two budgets.
- #47: f64 PageRank, identical bytes at 1, 4, 16 threads, in each regime, at budgets of 2,
  8, 16 GiB and unlimited, and on a view either way; f32 only on request.
- #48: component labels identical across threads, Afforest vs streaming union-find,
  budgets and views.
- #93: PageRank, components and communities identical on a view as mask or materialized,
  and after `save()`.
- #45: the result hash is the same in every regime the planner can choose.

## Consequences

- f64 state is the default wherever the result is a float; f32 is offered, never chosen.
- Reductions that carry a result run over reduction blocks of node ranges that are a
  function of the graph, never of the file's blocks, the chunk size, the thread count,
  the budget or the storage format (ADR 0028, A2; `docs/spec/eg-adjacency.md` § 4). For a
  file built by appends, results depend on its batches, not on how its runs are merged
  (ADR 0028, S4; `design/DECISIONS.md`). No float reduces over these blocks: a float
  that carries a result goes through the exact accumulator, whose value does not depend
  on the grouping. So the blocks, which a mask and a materialized view cut differently,
  change no bit.
- Leiden counts its draws by a node's place among the nodes with edges, and compares
  labels only by their order ("Reproducibility"). A view runs as a mask or materialized
  with the same result. A kernel that draws random numbers counts them the same way.
- The case that could narrow the promise is **out-of-core Leiden** (#60, acceptance:
  "deterministic mode works out of core with the same hash as in RAM"). Streaming
  aggregation combines (community, community, weight) triples in bucket order; with
  non-integer weights the totals are exact only with the fixed-point scheme not yet built.
- **Not to implement** (the review of 2026-11-26, [sota-review-2026-11.md](../research/sota-review-2026-11.md) (index in [docs/research/README.md](../research/README.md))): asynchronous execution, which processes active
  blocks in any order and lets updates of one block feed the next (ACGraph, Grafu), for
  kernels whose result depends on that order: PageRank, Leiden, label propagation.
  An asynchronous I/O pipeline is allowed: reductions follow fixed node blocks, not
  tasks, so the order in which reads complete does not change a result.
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

#267 (the exact contract: views, stored order, hysteresis, quality cost), #54 (the tests across threads, platforms, budgets and regimes), #60 (the case
that would narrow it).
