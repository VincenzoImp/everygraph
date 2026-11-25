---
title: One portability lint shared by every portable crate
milestone: M2 · Format and ingest
labels: type:infra, area:engine, P2, risk:low, size:S
---
**Why.** `crates/everygraph-core/clippy.toml` holds the portability rule (no `std::fs`, threads, clocks, …). The ingest and format crates must also reach the browser, but copying the core's 141-line list with core-specific messages would rot (text reader PR #152). The allocation lint of {{#m2-04-arrays-accountant}} (PR #154) covers only the kernel crates for now.

**Scope**
- [ ] One generated or shared list for every crate that must build for wasm32 (core, format, ingest, kernels, community, map, query), with crate-neutral messages; the portability canary checks each crate
- [ ] Extend the allocation lint to `everygraph-ingest`, `-format` and `-io` once their M2 issues have merged

**Acceptance**
- Adding a forbidden call to any portable crate fails CI with the shared message; the canary proves it per crate
