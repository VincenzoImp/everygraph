---
title: M3 close: dogfood pass, cleanup and test audit, report
milestone: M3 · Engine and kernels
labels: type:infra, area:engine, P1, risk:low, size:M
---
**Why.** Every milestone ends with a dogfood pass, a cleanup issue, the official measurement window and a report (CLAUDE.md § 6, § 10). {{#m2-31-cleanup-audit}} does the cleanup for M2 and {{#m3-22-performance-pass}} holds the speed gaps and the window of M2 and M3. Nothing owns the rest: no agent has yet used the product as a user would.

**Scope**
- [ ] Dogfood pass: an agent that has not written the code uses `everygraph` (command line) and the Python package on real datasets, tier 1 and tier 2, under a memory limit, following each user story of `docs/api-sketch.md` as far as the product goes. Every wrong result, wrong message, awkward step and disagreement with `docs/api-sketch.md` or `docs/ux-contract.md` becomes an issue file
- [ ] Cleanup of the M3 crates (`everygraph-kernels`, and what M3 added to core, io and format), in the shape of {{#m2-31-cleanup-audit}}: lines per module, duplicated helpers, the `cargo mutants` audit with {{#m3-17-cargo-mutants}}'s tooling, tests that no mutant can fail rewritten or removed. Duplicates known today (main at 00558d4):
  - `u64` to `usize` with an `expect`, six product copies: `ix` (`kernels/edge_map/mod.rs`), `to_usize` (`format/adjacency/decode.rs`, `format/adjacency/merge.rs`, `ingest/build/transpose.rs`, `oracle/lib.rs`), `index` (`ingest/ids/hash.rs`). One function in `everygraph-core`, beside `widen` and `usize_of`
  - `unite` (union-find with atomic cells): `kernels/components.rs` and `community/level.rs`
  - `check_adjacency`: `format/verify.rs` and `ingest/build/mod.rs`; the build should call the verifier's check, or say why it cannot
  - test helpers: `splitmix` (`kernels/tests/framework.rs`, `kernels/tests/kernels.rs`, `ingest/tests/support`, `ingest/src/ids/tests.rs`, `memtest/src/graphs.rs`) and `planted` (`determinism/src/lib.rs`, `community/tests/leiden.rs`, `community/tests/hierarchy.rs`, `format/tests/results.rs`), each to one test-support place
  - `Pieces` in `kernels/scc.rs` (atomics in pieces of at most one allocation each): a segmented vector of atomics in `everygraph-core`, beside `SegVec`
- [ ] The report `docs/reports/M2.md` and `docs/reports/M3.md`: each exit criterion with its value and result file, what slipped and why, decisions changed, source and test lines per crate
- [ ] `docs/STATE.md` and `docs/ROADMAP.md` say what the reports found

**Done when.** The dogfood issues are filed, the cleanup pull request is merged, and both reports are in `docs/reports/`.

**Depends on** {{#m3-02-planner}}, {{#m3-03-semi-external}}, {{#m3-10-no-oom-ci}}, {{#m3-11-determinism-ci}}, {{#m3-22-performance-pass}}
