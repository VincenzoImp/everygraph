---
title: Sampled centralities: betweenness, closeness, harmonic
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P2, risk:medium, size:M
---
**Why.** Analysts ask for betweenness constantly, and exact algorithms are infeasible beyond ~10^6 edges. Not covered by the lab.

**Scope**
- [ ] Harmonic and closeness centrality via multi-source BFS batches ({{#m3-06-traversal}}) or HyperLogLog-based neighborhood functions
- [ ] Degree and eigenvector centrality for completeness
- [ ] Every approximate result follows the approximate-results contract (`design/DECISIONS.md`, "Approximate results"; `docs/api-sketch.md` § 2.8): `.approximate`, `meta["approximate"]` with method, seed, samples, the guarantee's kind, ε and δ or the RSD, and runs, the same keys in the provenance record; precision parameters (registers m, ε, δ, the sample schedule) are never derived from the budget or the thread count
- [ ] Distances, closeness and harmonic centrality through HyperBall (HyperLogLog counters; a union is a per-register max, so the order of unions does not matter), as an explicitly approximate kernel (`docs/research/memory-bounded-algorithms.md`). Bit identity for any thread count, chunking, regime and platform needs four rules (decided 2026-11-25, `docs/research/README.md` R2; `docs/research/sota-approx-rust.md` § 3 A, challenge 2):
  - a Jacobi double buffer: counters of hop t are read while those of t + 1 are written (in-place updates mix radii and make the sums depend on the schedule);
  - a seeded, fixed hash of the node id, with the seed of run r derived from (seed, r) through {{#m2-27-counter-rng}};
  - an estimator without libm: Ertl's improved raw estimator uses only +, ×, √ and the constant 1/(2 ln 2), and can be table-driven over C₀/m; the classic small-range correction uses `ln`, which differs across glibc, macOS and wasm;
  - termination by stabilization, never a fixed hop count ("any alternative termination condition may lead to arbitrarily large mistakes", Boldi–Vigna); per-node f64 sums accumulate in hop order.

  The printed error is what is true: the RSD per counter (1.04/√m) for reachable-set sizes; for harmonic and closeness, "no proven bound", with an empirical spread over R ≥ 2 runs when asked for (Boldi–Vigna control the error the same way). HyperBall stays the default over ADS+HIP, which would cost about 30–60× the bytes at equal per-node error (≈ 110 GB for Friendster by extrapolation from Oasis's soc-LJ density, against ≈ 6.3 GB for HyperBall at m = 64 plus 1.05 GB of f64 sums, arithmetic in the report)
- [ ] HyperBall over budget (decided 2026-11-25, `docs/research/README.md` R3): when n·m registers exceed the budget, partition the counters Oasis-style (FAST'25): one destination block of counters in RAM, the source blocks' counters and the in-edges of that block streamed (the in-adjacency, D6, grouped by destination block). Max-union makes the result independent of the partition count, so the budget changes only the time. Experiment: Friendster, m = 64, budgets 8 GB and 2 GB, hash identity and time ({{#m3-03-semi-external}})
- [ ] **Betweenness by path sampling, not source sampling** (changed 2026-11-25, `docs/research/README.md` R1; `docs/research/sota-approx-rust.md` § 3 A, challenge 1). The earlier text — Riondato–Kornaropoulos "batched with multi-source bitset BFS" — does not fit betweenness: multi-source BFS batches reachability, but Brandes-style source sampling also needs σ and δ per (source, node), about 20 B per node per source, ≈ 1.3 GB per concurrent source on Friendster and ≈ 84 GB for a 64-wide batch (arithmetic). Instead:
  - KADABRA's balanced bidirectional BFS path sampler (a sample visits about m^½ edges; KADABRA about 100× faster than RK on undirected graphs at λ = 0.005, 48 cores, their machine), on the random-access path of {{#m3-01-edge-map}} and, for directed graphs, the in-adjacency (D6);
  - SILVAN's fixed geometric schedule m_i = m₀θ^i and its non-uniform bounds (at least 20% fewer samples than KADABRA on all 18 of its graphs, under half on 14; 64 cores, their machine); stopping is tested only at the schedule's points, which are data-defined;
  - **integer** hit counts, order-free, so the result is bit-identical for any thread count and budget, with each sample's RNG keyed by (seed, sample index) ({{#m2-27-counter-rng}});
  - SILVAN's c = 25 Rademacher sums stored sparsely (only nodes on sampled paths): dense as i32 on Friendster they would take ≈ 6.6 GB (arithmetic);
  - a **top-k mode** with its guarantee for `top()`, and "indistinguishable from 0" printed for nodes with an estimate below ε: an additive-ε bound says nothing about the order of a flat tail
- [ ] Multi-source bitset BFS stays for exact sampled closeness and harmonic pivots, whose distance sums are integers (popcount × level) ({{#m3-06-traversal}})
- [ ] The "orkut in under 60 s" target has no outside support (no orkut row in KADABRA, SILVAN or Bavarian): confirm or restate it with the first measurement

**Acceptance**
- On graphs where exact betweenness is feasible (com-dblp, com-youtube, amazon from SNAP; exact Brandes is not feasible on lj or orkut), the top-100 ranking **of the top-k mode** overlaps ≥ 90% with exact at the default budget, and the maximum observed error is at most the printed ε
- Betweenness and HyperBall hashes identical at 1, 8, 16 and 32 threads and at two budgets; HyperBall's estimator bit-equal on x86-64, aarch64 and wasm32 over C₀/m ∈ [0, 1], and lj hashes equal at 1 and 16 threads, native and wasm
- orkut approximate betweenness in < 60 s (to be confirmed or restated, above), reported as time to ε = 10⁻³ at 16 threads

**Pointers.** `docs/research/sota-approx-rust.md` § 2 A, § 3 A, § 4 challenges 1–4, 8–9 (HIP-on-HyperBall and UltraLogLog registers as later experiments); Borassi–Natale (ESA'16); Pellegrina–Vandin (TKDD'22); Boldi–Vigna (arXiv 1308.2144); Ertl (arXiv 1702.01284); Yang et al., Oasis (FAST'25).

**Depends on** {{#m3-06-traversal}}, {{#m2-27-counter-rng}}, {{#m2-26-core-math}}
