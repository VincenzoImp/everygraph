---
title: Compile weight of the tests: one A/B per remedy on the CI runners
milestone: M3 · Engine and kernels
labels: type:infra, area:foundations, P2, risk:low, size:M
---
**Why.** The CI build job takes 11–14 minutes and is bound by the CPU: 3,331 s of CPU in 847 s of wall time on four cores (the CI audit of 2026-11-27, GitHub-hosted runners). A few units take most of it: the `everygraph-memtest` library 330 s, the determinism suite's test 293 s, the Leiden test of `everygraph-community` 230 s, the examples 565 s in all. The pull-request CI must stay under 15 minutes (CLAUDE.md § 6), and the build is the part every job waits for.

**Method.** Each remedy is one interleaved A/B on GitHub's free runners (no paid runners, no self-hosted runner): the same commit with and without it, at least three runs each, `cargo build --timings` for the CPU per unit and the job's wall time.
- [ ] One test binary per crate (`tests/*.rs` gathered into one `tests/all/main.rs` with modules), for the crates with the most test binaries
- [ ] Per-crate `opt-level` in the dev and test profiles: 0 for crates whose tests do not need speed, a higher level only where a test needs it
- [ ] `sccache` with the GitHub Actions cache backend, inside the free cache quota
- [ ] The heaviest units: what makes them heavy (generic instantiations, large `proptest` strategies, macros), from `--timings` and `cargo llvm-lines`; fewer monomorphizations where a test instantiates a kernel over many types for no added coverage
- [ ] The examples: built only by the jobs that run them

**Done when.** Each remedy is kept or dropped on its A/B, with the runs named in the PR; the build job's CPU time and wall time before and after are in the PR; CONTRIBUTING.md "CI" states what was kept.

**Pointers.** CONTRIBUTING.md "CI", rules 3 and 4; `.github/workflows/ci.yml`; `Cargo.toml` profiles.
