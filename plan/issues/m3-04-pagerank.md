---
title: PageRank family
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P0, risk:low, size:M, lab-evidence
---
**Scope**
- [ ] Pull-based PageRank on undirected and directed graphs (uses in-adjacency), damping, dangling-mass handling, max iterations and tolerance
- [ ] Weighted and personalized variants (single source and seed set)
- [ ] Multiplicity is read from the `.events` sequence or a per-pair column, measured both ways (moved from {{#m2-10-property-columns}}; `docs/spec/eg-columns.md` § 6): an interleaved A/B of weighted PageRank with `weight=True` and no weight column, on one temporal graph with repeated pairs, decides whether the build stores a per-pair multiplicity column
- [ ] The formula of `docs/spec/graph-semantics.md` § 5.1 (S1): `x(v) = (1 − α)·t(v) + α·(Σ_u x(u)·A[u][v]/s(u) + t(v)·Σ_{u dangling} x(u))`, `s(u) = Σ_v A[u][v]`; a kept loop is `A[v][v]` = 2w undirected, w directed, and a node whose only edge is a loop is not dangling; dangling mass follows the teleport vector `t` (uniform, or over the `personalize` set)
- [ ] Weights as `docs/spec/graph-semantics.md` § 4.1 and § 4.4: the summed weight, or the multiplicity without a weight column; zero allowed; a negative, NaN or ∞ weight is `InvalidInput` (3), decided from the column statistics before any work (§ 2.7, § 7)
- [ ] f32 and f64 state. f64 is required for any run compared with LDBC Graphalytics or published beside other tools ({{#m8-03-public-benchmarks}}, {{#m1-20-ldbc-graphalytics}}): the LDBC PageRank specification requires f64, and our S1 formula with uniform teleport equals LDBC's in f64 (`docs/research/memory-bounded-algorithms.md`, addendum item 17). f32 stays an option (~10–15% faster and halves the state; lab: 0.37 vs 0.43 s on orkut; on the default storage 0.27 / 0.31 s at 16 threads on orkut (#118))
- [ ] Deterministic summation order: every reduction (the rank sums, and the dangling mass in particular) runs over fixed reduction blocks defined by node ranges, independent of the scan's chunk size, the thread count and the storage format (raw or compressed). The oracle found that the lab prototype's dangling-mass sum follows the chunking: on 56 nightly graphs with isolated nodes its bits differ between chunks of 64 and 65,536 (PR #131); no reduction relies on rayon's floating-point `sum`, whose reduction tree is unspecified. The dangling mass and the other global scalars (residual, totals) go through the exact accumulator of {{#m2-26-core-math}}, so the PR #131 class of bug cannot recur for a new scalar (`docs/research/README.md` N6)
- [ ] Convergence semantics documented against NetworkX and igraph (NetworkX compares L1 error with `N·tol`, which stops after one iteration for N > 2 M — a trap the NetworkX backend must reproduce, the native API must not)
- [ ] Returns an Arrow column aligned with node ids; top-k helper
- [ ] Eigenvector centrality, exact, by the same power iteration, reductions and stopping rule as PageRank (moved here from {{#m3-09-centrality}}, whose sampled centralities come after 0.1); public name `g.eigenvector()` (`docs/api-sketch.md` § 7). Acceptance: equal to NetworkX `eigenvector_centrality_numpy` within 1e-9 (L∞) on the oracle graphs, and byte-identical at 1 and 16 threads

**Acceptance**
- Matches NetworkX/igraph within 1e-9 on the oracle graphs with equal parameters
- Weighted PageRank on a per-pair weight column (indexed by entry offset; on orkut, which has no repeated pairs, the same as per event) within 15% of the unweighted kernel on orkut (moved from {{#m2-10-property-columns}}, which built the column format)
- On graphs with kept loops, matches igraph within 1e-9 (NetworkX puts w, not 2w, on an undirected loop and matches only when given the loop weight 2w); probe: undirected [0.475, 0.2625, 0.2625], loop weight 3 [38/63, 25/126, 25/126], directed [0.480056, 0.254024, 0.265920] (`docs/spec/graph-semantics.md` § 5.1, § 5.6)
- orkut, 20 iterations, f64: ≤ 0.5 s at 16 threads; identical bytes at 1, 4, 16 threads
- Re-running an identical call on a graph file returns the stored result in milliseconds (moved from {{#m2-12-artifact-cache}})
- Runs in all three regimes; the out-of-core cost against in RAM (D11's "~10%" for Friendster PageRank f32 under 8 GB and 2 GB) is restated only from interleaved in-RAM and out-of-core rounds with a ratio interval ({{#m3-19-harness-interleave}}): 10% is inside what a fixed run order and drive state can produce (added 2026-11-25, `docs/research/README.md` M6; `docs/research/sota-measurement.md` § 3.4)
- With f64 state the result has identical bytes (the hash of {{#m3-11-determinism-ci}}) at 1, 4 and 16 threads, in each of the three regimes, at memory budgets of 2, 8 and 16 GiB and unlimited, and on a view whether it is held as a mask or materialized; f32 state is used only when `precision="low"` is asked for, never chosen by the planner (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises")

**Pointers.** `lab/b1-core` kernels; top-5 on orkut matched NetworKit to ~15 digits.

**Left after PR #220 (2026-11-26).** PR #220 built `everygraph_kernels::pagerank` (f32 and f64 state, weighted, personalized, eigenvector centrality); its description lists the evidence per box. Closed on 2026-11-26 on its tests and exploratory evidence (CLAUDE.md § 6). The time and the hashes come from the M3 milestone window (`everygraph-bench run kernels --official --data /home/tux/graphlab/data`). The out-of-core cost from interleaved rounds ({{#m3-19-harness-interleave}}) and the weighted run on a per-pair column against the unweighted kernel are {{#m3-22-performance-pass}}'s.

**Depends on** {{#m3-01-edge-map}}, {{#m2-08-csr-build}}, {{#m2-10-property-columns}}
