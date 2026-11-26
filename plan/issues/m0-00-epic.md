---
title: M0 epic: foundations and decisions
milestone: M0 · Foundations and decisions
labels: type:epic, area:foundations, P0
---
Tracking issue for milestone M0.

**Goal.** Anyone can clone the repository, build it and run the tests with one command. Every design decision in `docs/DESIGN.md` is ratified as an ADR or has an experiment assigned in M1.

**Exit criteria**
- [x] Cargo workspace builds on Linux, macOS and Windows, and `everygraph-core` builds for `wasm32-unknown-unknown` ({{#m0-01-workspace}}, {{#m0-02-ci}})
- [x] Benchmark harness reproduces the lab headline numbers on an idle machine ({{#m0-03-bench-harness}}; `bench/results/headline/`). Porting every lab script ({{#m0-07-lab-repro}}) is dropped: the prototypes are frozen
- [x] Differential-test oracle runs in CI ({{#m0-05-oracle}})
- [x] Decisions D1–D25 ratified as ADRs ({{#m0-06-adrs}})
- [x] Name, licence and patent checks closed ({{#m0-08-licence-governance}}, {{#m0-10-patent-check}}; what is left of the name check, {{#m0-09-name-trademark}}, is in {{#m8-10-legal-opinions}})
- [x] API sketch reviewed ({{#m0-11-api-sketch}})

**Order.** {{#m0-01-workspace}} → {{#m0-02-ci}} → the rest in parallel. The legal checks ({{#m0-09-name-trademark}}, {{#m0-10-patent-check}}) have external lead time: start them on day one.

**Closed** with `docs/reports/M0.md`.
