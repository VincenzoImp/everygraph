---
title: One portability lint shared by every portable crate
milestone: M2 · Format and ingest
labels: type:infra, area:engine, P2, risk:low, size:S
---
**Why.** `crates/everygraph-core/clippy.toml` holds the portability rule (no `std::fs`, threads, clocks, …). The ingest and format crates must also reach the browser, but copying the core's 141-line list with core-specific messages would rot (text reader PR #152). The allocation lint of {{#m2-04-arrays-accountant}} (PR #154) covers only the kernel crates for now.

**Owner of the shared lint configuration** (decided 2026-11-26): this issue writes and owns it; {{#m2-31-cleanup-audit}} only applies it in its cleanup.

**Scope**
- [ ] One generated or shared list for every crate that must build for wasm32 (core, format, ingest, kernels, community, map, query), with crate-neutral messages; the portability canary checks each crate
- [ ] Extend the allocation lint to `everygraph-io`. The same lint for `everygraph-format` and `everygraph-ingest` is {{#m2-31-cleanup-audit}}'s box, not this issue's
- [x] Done by PR #210 (`tools/numerics_lint.py` writes the list into every product crate's `clippy.toml`; CONTRIBUTING, "The numerics rule"). The numerics rules of {{#m3-11-determinism-ci}} that a lint can check, in the same shared list (decided 2026-11-25, `docs/research/README.md` N2; `docs/research/sota-numerics.md` § 4, rules R2, R3, R8, R9): `disallowed-methods` for the std transcendentals and `powi`/`powf` outside the pinned math module ({{#m2-26-core-math}}), `mul_add_relaxed`, `algebraic_*` and fast-math or approximate intrinsics, `is_x86_feature_detected!`, `f64::max`/`min` where signed zeros can tie (`total_cmp` instead); `disallowed-types` for `std::collections::hash_map::RandomState` and the default hashers of hashbrown, foldhash and ahash; and in `deny.toml`, `fastrand`, `getrandom` and `rand` outside dev-dependencies. Each entry's message names the replacement

**Acceptance**
- Adding a forbidden call to any portable crate fails CI with the shared message; the canary proves it per crate
- The numerics lists flag the lab's known violations when pointed at them: `lab/e-map/gmap/src/layout.rs` l. 139–140 (`cos`/`sin`) and `crates/everygraph-oracle/src/generators.rs` l. 282 (`powf`) (`docs/research/sota-numerics.md` § 3.1)

**Depends on** {{#m2-26-core-math}}, {{#m2-27-counter-rng}}
