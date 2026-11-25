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
- With f64 state the result has identical bytes (the hash of {{#m3-11-determinism-ci}}) at 1, 4 and 16 threads, in each of the three regimes, at memory budgets of 2, 8 and 16 GiB and unlimited, and on a view whether it is held as a mask or materialized; f32 state is used only when `precision="low"` is asked for, never chosen by the planner (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises")

**Pointers.** `lab/b1-core` kernels; top-5 on orkut matched NetworKit to ~15 digits.

**Depends on** {{#m3-01-edge-map}}
