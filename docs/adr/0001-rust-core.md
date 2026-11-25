# ADR 0001 — Rust core, no C/C++ dependencies in the core

- **Status:** accepted
- **Decision:** D1 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The engine must be fast on a workstation, embeddable from Python and Rust, and the same
code must run in a browser (D2, D22). A C or C++ dependency in the core complicates the
WebAssembly build, the wheels and the licence review.

## Decision

The core is written in Rust and has no C or C++ dependencies. Native libraries are allowed
only behind optional features outside the core (for example webgraph-rs, ADR 0005).

## Evidence

- **Speed of a Rust prototype against the strongest C++ reference** (FINDINGS "What the
  comparison says"; idle; orkut; uncompressed arrays; each tool runs its own algorithm for
  the task; `lab/b1-core/results/bench/kernels.jsonl`,
  `lab/a-baselines/results/baselines.jsonl`):
  - PageRank, 20 iterations, f64: 0.43 s (lab, 16 threads) vs 2.79 s (NetworKit, 32
    threads) — 6.5×.
  - Connected components: 13 ms (Afforest, 16 threads) vs 247 ms (NetworKit parallel
    components, 32 threads) — 19×.
  - k-core: 0.28 s (8 threads) vs 1.23 s (32 threads) — 4.4×.
  - NetworKit was run at 1 and 32 threads; 32 was its faster setting for these tasks.
- **Portability** (FINDINGS § G; shared): the same Rust code compiled to wasm32 and wasm64
  ran in Chromium and Node with the same PageRank sum and maximum and the same component
  count as native on lj (`lab/g-wasm/results/native.jsonl`, `chromium.jsonl`,
  `node.jsonl`).
- **Cost of a native dependency** (FINDINGS § C): webgraph-rs's non-optional native
  dependencies make a WebAssembly build unlikely. This was judged from its dependency
  tree, not tried.

What this evidence does not show: that a C++ core would be slower. The comparison is
between implementations, and the lab kernels ran on raw arrays, not on the compressed
default (ADR 0004, #118).

## Consequences

- No `cc`/`cmake` build steps or `-sys` crates in the core crates; a reviewer rejects a
  pull request that adds one without an ADR.
- SIMD is written in Rust (intrinsics or portable SIMD), not linked from C.
- Speed claims against NetworKit are restated for the production kernels (#47, #48) on the
  default storage before they are made public.

## What would change our mind

- A capability needed for v0.1 exists only as a C/C++ library and no Rust implementation
  comes within 2× of its speed on the lab graphs.
- The production kernels (#47, #48), on an idle machine at equal thread counts, end up
  slower than NetworKit on orkut.
