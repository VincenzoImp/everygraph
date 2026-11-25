# ADR 0003 — Dense u32 node ids by default

- **Status:** accepted
- **Decision:** D3 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Graph kernels are dominated by memory traffic over adjacency targets and O(n) state. The
width of a node id sets both. Most target graphs have fewer than 2^32 nodes; the largest
public ones (web crawls) do not.

## Decision

Node ids are dense u32 by default. Offsets on disk are u64 (or Elias–Fano coded). A build
with u64 ids for graphs of 2^32 nodes or more comes later (#114).

## Evidence

- **Cost of u64 targets** (FINDINGS "Idle-machine re-runs", B1 table; idle; orkut;
  PageRank 20 iterations, 32 threads, heap-loaded, uncompressed;
  `lab/b1-core/results/bench/kernels.jsonl`, label `probe`): f32 state 0.70 s with u64
  targets vs 0.375 s with u32 (+87%; FINDINGS rounds to 0.70 vs 0.38, +85%); f64 state
  0.80 vs 0.47 s (+71%). One kernel, one graph.
- **The compressed form does not care** (FINDINGS § C; shared): gap codes give identical
  bits/edge with ids shifted by 2^33 (`lab/c-compression/results/idwidth.json`); offsets
  must be u64 or Elias–Fano, which cost 0.13–0.16 bits/edge (FINDINGS § C).
- wasm32 addresses 4 GiB in total (ADR 0002), which u64 ids would fill twice as fast.

## Consequences

- The id type is a type parameter of the core from the start, so the u64 build of #114 is
  a second instantiation, not a rewrite.
- Until #114, graphs with 2^32 nodes or more are unsupported. How the ingest refuses such
  an input (rather than overflowing) is not yet written into an ingest issue.

## What would change our mind

- #114 shows a u64-id build within 10% of u32 on orkut PageRank (idle, 16 threads), which
  would make a single build simpler than two.
- Graphs with 2^32 nodes or more become a v0.1 target.
