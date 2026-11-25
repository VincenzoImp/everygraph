---
title: PageRank family
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P0, risk:low, size:M, lab-evidence
---
**Scope**
- [ ] Pull-based PageRank on undirected and directed graphs (uses in-adjacency), damping, dangling-mass handling, max iterations and tolerance
- [ ] Weighted and personalized variants (single source and seed set)
- [ ] f32 and f64 state (f32 is ~10–15% faster and halves the state; lab: 0.37 vs 0.43 s on orkut)
- [ ] Deterministic summation order via the framework's fixed chunking
- [ ] Convergence semantics documented against NetworkX and igraph (NetworkX compares L1 error with `N·tol`, which stops after one iteration for N > 2 M — a trap the NetworkX backend must reproduce, the native API must not)
- [ ] Returns an Arrow column aligned with node ids; top-k helper

**Acceptance**
- Matches NetworkX/igraph within 1e-9 on the oracle graphs with equal parameters
- orkut, 20 iterations, f64: ≤ 0.5 s at 16 threads; identical bytes at 1, 4, 16 threads
- Runs in all three regimes
- Identical bytes whichever regime the planner picks (in memory, edges on disk, state partitioned) and on a view run as a mask or materialized: tested by forcing each (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises")

**Pointers.** `lab/b1-core` kernels; top-5 on orkut matched NetworKit to ~15 digits.

**Depends on** {{#m3-01-edge-map}}
