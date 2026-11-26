---
title: k-core decomposition and degree statistics
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P1, risk:low, size:S, lab-evidence
---
**Scope**
- [ ] Degree columns `in_degree`, `out_degree`, `degree` (distinct neighbors; `degree` counts each neighbor once whatever the direction) and `weighted`; on a view they are computed by a planned pass. Also, exact histogram, quantiles, isolates — computed at ingest and stored
- [ ] k-core designed after **Liu, Dong, Gu, Sun (SIGMOD'25)** (decided 2026-11-25, `docs/research/sota-kernels.md` challenge 3): an explicit active set (O(n + m), no per-round rescan), sampled degree counters for hubs (Las Vegas: still exact, restart on a detected error; samples from a counter-based RNG), vertical granularity control with local queues, hierarchical buckets (the bucketing primitive of {{#m3-01-edge-map}}). Liu et al. report Friendster 3.68 s and Twitter 2.72 s at 192 hyper-threads (their machine), against our lab's 7.65–8.74 s and 5.26–5.50 s at 16 threads; our PKC-style peel is already far faster than the PKC they measured (Twitter 75.6 s), so their ratio over PKC does not transfer. Lab history: the first level-synchronous version scaled *negatively* (1.39 s at 8 threads, 1.93 s at 32); PKC-style peeling 0.28 s at 8
- [ ] Interleaved A/B in the lab before the port: twitter-sym, Friendster and orkut (the hard case: 5,667 peeling rounds) at 8, 16 and 32 threads; time, scaling past 8 threads, identical core numbers
- [ ] Core-number column, shell sizes, k-core subgraph as a view
- [ ] Core numbers ignore kept loops: the k-core is the largest induced subgraph of the undirected projection with |N'(v)| ≥ k (distinct neighbours other than v); multiplicity and weight do not enter (`docs/spec/graph-semantics.md` § 5.2, S2)
- [ ] Degrees as `docs/spec/graph-semantics.md` § 3: a kept loop adds 1 to `in_degree` and `out_degree` and 2 to `degree`; `weighted` is the strength in the undirected projection, a kept loop adding 2w, in the exact types of § 4.3; negative, NaN and ∞ weights are summed, never refused (§ 4.4)
- [ ] Note: k-core does not scale past 8 threads in the prototype (0.28 → 0.30 → 0.37 s at 8/16/32). The cause is algorithmic (hub contention and per-round rescans, `docs/research/sota-kernels.md` § 3, D13 row), so capping threads is not the fix; the design above is
- [ ] A bounded-memory path for core numbers: SemiCore* (Wen et al., ICDE 2016; exact, unique fixpoint, O(n) memory, re-reads only nodes whose estimate may change) over the edge-map's semi-external source; identical result to the in-RAM peel (`docs/research/memory-bounded-algorithms.md`). Without it the planner refuses k-core when the graph does not fit, which leaves promise 2 uncovered
- [ ] Out of core, k-core may skip inactive lists and run in any order: the answer is the unique fixpoint of a monotone update, so the output is identical by construction (Seraph-style active-list decoding, Gem-style bound pruning; {{#m3-03-semi-external}}; `docs/research/memory-bounded-algorithms.md` addendum § F 5, added 2026-11-25). Identity with the in-RAM result is asserted in the tests

**Acceptance**
- Core numbers equal NetworkX on oracle graphs (with kept loops, NetworkX run on the loop-free graph; igraph counts loops and differs, `docs/spec/graph-semantics.md` § 5.2) and are the same with loops kept and dropped; max core 360 (lj), 253 (orkut), 304 (Friendster)
- orkut ≤ 0.3 s (needs a stable measurement: #118 saw 0.22–0.36 s at 16 threads in every configuration); Friendster ≤ 10 s

**Left after PR #218 (2026-11-26).** PR #218 built `everygraph_kernels::kcore` (after Liu et al., with hierarchical buckets in the edge-map framework) and `everygraph_kernels::degrees`; its description lists the evidence per box. Closed on 2026-11-26 on its tests and exploratory evidence (CLAUDE.md § 6). Where the rest went: the orkut and Friendster times, 1.4–1.9× the lab in exploratory runs, {{#m3-22-performance-pass}}; SemiCore* and out-of-core skipping, {{#m3-03-semi-external}}; the degree histogram, quantiles and isolates stored at ingest, {{#m2-34-build-columns}}; degree columns and the k-core subgraph on a view, {{#m7-04-views}}.


**Depends on** {{#m3-01-edge-map}}
