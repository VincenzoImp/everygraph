# ADR 0012 — One edge-map framework

- **Status:** accepted, partly validated
- **Decision:** D12 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Each kernel must run over in-memory slices, mmap, sequential streams (ADR 0011),
compressed blocks (ADR 0004) and in WebAssembly (ADR 0002). Writing each kernel once per
source multiplies code and the chance that results differ between regimes (ADR 0014).

## Decision

One edge-map framework: each kernel body is written once and runs over every source,
called as many short calls so WebAssembly engines optimize it.

## Evidence

- **Same body, two sources — but copied, not shared**: in the prototype the per-vertex
  loop of in-RAM PageRank (`lab/b1-core/crates/glab-core/src/kernels.rs`, `pagerank`) and
  of semi-external PageRank (`lab/b1-core/crates/glab-core/src/extmem.rs`,
  `pagerank_semiext`) is the same code, the second receiving each chunk's targets from a
  stream. No framework was built. Components used two different algorithms (Afforest in
  RAM, streaming union-find out of core), so for them "one body" was not shown.
- **WebAssembly tiering** (FINDINGS § G; shared; lj, PageRank 20 iterations, 1 thread,
  one long call; `lab/g-wasm/results/chromium.jsonl`): 25.3 s with Chromium's defaults
  against 5.5 s with the optimizing tier forced (`--no-liftoff`) — 4.6×. DESIGN and
  FINDINGS say "up to 9×"; that does **not** follow from these two numbers. 25.3 s
  against native single-threaded 2.81 s (`lab/g-wasm/results/native.jsonl`) is 9.0×, but
  that mixes tiering with wasm-vs-native.
- Not measured: the overhead of a generic framework over hand-written kernels; compressed
  blocks as a source.

## Consequences

- #44 builds the framework; its acceptance is the missing measurement: PageRank, WCC and
  BFS written once, byte-identical over all four sources, ≤ 5% overhead against the
  `lab/b1-core` kernels on orkut, compiled to wasm32 with native-identical output.
- Kernels that need a different algorithm per regime (components) declare their variants
  to the planner, which may switch only between variants that give the identical result
  (ADR 0014).

## What would change our mind

- #44 measures more than 5% overhead on orkut that cannot be removed, or more than half of
  the core kernels need regime-specific algorithms, so that the framework saves little.

## Closed by

#44 (edge-map framework).
