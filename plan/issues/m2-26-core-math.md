---
title: Core numerics: one pinned math module, an exact accumulator, canonical floats
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P1, risk:medium, size:M
---
**Why.** "The same result on any platform" (D14) fails wherever a result passes through a platform maths routine. `f64::sin` and `cos` go to glibc on Linux, to the `libm` crate on `wasm32-unknown-unknown`, to Apple's libm on macOS and to musl on `*-linux-musl`. Those libraries return different bits: largest known binary64 errors of 0.516 ulp for glibc 2.44 `sin` against 0.831 for musl 1.2.6 (Gladman et al. 2026, Table 3). glibc also dispatches at run time by CPU feature, so one binary can differ between two x86-64 machines, and glibc 2.27 and 2.36 return different `sin` for the same input (`docs/research/sota-numerics.md` § 2, CORE-MATH slide 3). Code that reaches results already calls them: the lab layout (`lab/e-map/gmap/src/layout.rs` l. 139–140, `cos`/`sin`), the tile zoom (`tiles.rs` l. 134, `log2`; l. 322, `powi(2)`), Leiden's tolerance (`0.1f64.powi(iter)`: `lab/b2-leiden` l. 846, `m1-13` l. 843, `m1-02` l. 1105) and the oracle's graph generator (`crates/everygraph-oracle/src/generators.rs` l. 282, `powf`). Identity does not need correct rounding, only one implementation compiled for every target (decided 2026-11-25, `docs/research/README.md` N4).

Two smaller hazards belong to the same module (N6). A global scalar summed over chunks — the dangling mass of PageRank was one (PR #131: bits differed between chunks of 64 and 65,536) — depends on the reduction blocks unless it is exact. And Rust leaves the sign of a NaN non-deterministic (RFC 3514), x87 returns alter payloads and the wasm full profile leaves NaN bits open, so equal results can hash differently.

**Scope**
- [ ] `everygraph_core::math`: the transcendentals result code needs (at least `sin`, `cos`, `exp`, `ln`, `log2`, `sqrt` wrappers where useful), wrapping the pure-Rust `libm` crate at an exact pinned version (`=x.y.z`), or own code for the few functions needed. The report infers from `libm` 0.2.16's source that it has no target-dependent float paths except on i586 (`src/math/arch/mod.rs`); confirm it by the golden vectors below. Avoid `pow`: `libm`'s comes from FreeBSD's `e_pow.c`, and FreeBSD 15.1's library shows 636 ulp (Gladman Table 3; that ours shares the error is an inference)
- [ ] Replace the lab's calls when their production issues port them: `x*x` for `powi(2)`, precomputed constants for `0.1^iter`, `ilog2`/`leading_zeros` for integer decisions such as the zoom level, the pinned module for the rest ({{#m5-01-layout}}, {{#m5-03-tile-builder}}, {{#m4-01-leiden-production}}); the oracle generator's `powf` now
- [ ] Golden vectors: 10^5 inputs per function, compared bit for bit on x86-64, aarch64 (qemu-user on the self-hosted runner), i686 and wasm32 (Node); the test is part of {{#m3-11-determinism-ci}}
- [ ] An exact accumulator for global scalars (2m or T, Q, the dangling mass, residuals, totals in map tiles): Neal's small superaccumulator or a three-limb Hallberg fixed point, mergeable per chunk in any order, so reduction blocks stop mattering for scalars. Tested against the `xsum` crate as a dev-dependency oracle. Cost: O(n) values per iteration at "less than twice" a plain sum (Neal, serial) against about 3.2 s per PageRank iteration on Friendster — well below 1% (arithmetic estimate in the report; to be measured, {{#m3-04-pagerank}})
- [ ] Canonical floats: one helper maps every NaN to the canonical quiet NaN (`0x7FF8…0`) and −0.0 to +0.0, used before results are written, put in derived sections or hashed; `total_cmp` where ±0 or NaN can tie; a float fold helper that starts from +0.0 (rule R7 of {{#m3-11-determinism-ci}})
- [ ] The version of the rule list (`numerics=N`) and the pinned math version recorded in provenance

**Acceptance**
- Golden vectors bit-identical on the four targets
- The dangling-mass sum has the same bits at chunk sizes 64 and 65,536 and at 1 thread, without relying on fixed reduction blocks
- A result containing −0.0 or a NaN produced on x86-64 and on wasm32 hashes the same

**Feeds.** {{#m3-11-determinism-ci}}, {{#m4-01-leiden-production}}, {{#m5-01-layout}}, {{#m5-03-tile-builder}}, {{#m3-09-centrality}} (HyperBall's estimator and sample sizes), {{#m3-04-pagerank}}, {{#m3-01-edge-map}} (global scalars through the exact accumulator).

**Pointers.** `docs/research/sota-numerics.md` § 3.1 (m3-11 and D17 rows), § 4 challenges 4, 6 and 8, rules R2, R6, R10, R14; Neal, arXiv 1505.05571; Hallberg and Adcroft, Parallel Computing 40 (2014).
