# ADR 0005 — webgraph-rs is an optional feature, not the core codec

- **Status:** accepted
- **Decision:** D5 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

BVGraph (WebGraph), through webgraph-rs, is the reference for compressing web graphs and
brings LLP reordering. The question was whether it should be the default codec.

## Decision

webgraph-rs is an optional feature: BV import and export, LLP ordering, and an archival
tier for web graphs. The default codec is our own (ADR 0004). The default build contains
no webgraph code.

## Evidence

From the codec bake-off (FINDINGS § C; shared machine; 32 threads unless noted;
`lab/c-compression/results/bakeoff.jsonl`, `lab/c-compression/results/summary.txt`):
- **Space**: on the web graph uk-2005 BV takes 2.11 bits/edge against 11.43 for Stream
  VByte; on social graphs the gap is small (orkut 15.41 vs 16.66; twitter-sym 13.92 vs
  16.11).
- **Decode speed**, sequential scan, Stream VByte ÷ BV: at 1 thread 18.7× (orkut), 14.6×
  (twitter-sym), 5.0× (uk-2005); at 32 threads 9.1×, 21.9×, 5.2×. DESIGN and FINDINGS
  summarize this as "7–20×"; that range is **not traced** — the files give 5.0–21.9×
  (9.1–21.9× on the social graphs).
- **PageRank iteration**: BV 0.18 s vs 0.051 s (orkut), 2.9 vs 0.85 s (twitter-sym).
- **Cold reads**, twitter-sym, 6 threads: BV is decode-bound at 0.19–0.50 G edges/s against
  2.67 for Stream VByte (`lab/c-compression/results/dev6_cold8g.jsonl`).
- **Reordering**: LLP cut orkut's BV size by 26% (15.25 → 11.32 bits/arc) and made raw
  PageRank 32% faster; LLP took about 9 minutes (FINDINGS § C;
  `lab/c-compression/results/dev6_order.jsonl`).
- **Project facts** (FINDINGS § C, not measurements): about 100 M arcs/s per core; four
  breaking releases in 12 months; non-optional native dependencies make a WebAssembly
  build unlikely (judged, not tried); licence Apache-2.0 OR MIT since 0.7.0.

## Consequences

- #42 builds the feature; its acceptance requires that the default build has no webgraph
  code or dependencies.
- Web-graph users who need the smallest files use the archival tier and accept slower
  decode; kernels run on the default codec after import.
- Hierarchy-based reordering (#23) is tested as a cheaper substitute for LLP.

## What would change our mind

- Web graphs become the main workload and #24 shows the 5× size difference matters more
  to users than decode speed.
- webgraph-rs gains a WebAssembly-capable pure-Rust build, a stable release line, and a
  codec decoding within 2× of Stream VByte on social graphs at 16 threads.
