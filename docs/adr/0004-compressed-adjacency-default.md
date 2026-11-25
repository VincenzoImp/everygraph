# ADR 0004 — Compressed adjacency is the default storage

- **Status:** accepted, partly validated
- **Decision:** D4 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The lab started from "raw CSR by default, compressed when it does not fit" (ADR 0000). The
codec bake-off reversed that for storage. But every kernel, community and point-query
number the product quotes was taken on uncompressed arrays (DESIGN § 5, item 6), so the
default format has never been measured end to end.

## Decision

Adjacency is stored compressed by default: delta gaps coded with Stream VByte in
node-aligned blocks of about 64K edges, Elias–Fano offsets, 4 KiB-aligned sections, with
our own codec (about 400 lines). A raw CSR is an in-RAM decode cache the planner may build.

## Evidence

What the first lab measured on the compressed form — all in the codec bench of
experiment C, on a shared machine (FINDINGS § C: "32T rows are official runs under the
lock", which serialised official runs while other work kept running):
- **Size**: 16.66 bits/edge on orkut and 16.11 on twitter-sym against 32.84 and 33.11 raw
  (index included); 11.43 against 34.70 on uk-2005 (`lab/c-compression/results/bakeoff.jsonl`,
  `lab/c-compression/results/summary.txt`).
- **Sequential scan, 32 threads**: twitter-sym 21,688 vs 13,623 M edges/s raw; uk-2005
  21,744 vs 12,347; orkut 9,773 vs 12,734. At 1 thread raw is faster on all three
  (orkut 4,715 vs 1,757). Same files.
- **One pull-PageRank iteration, 32 threads, f32**: orkut 0.051 s vs 0.026 s raw (raw 2×
  faster); twitter-sym 0.854 vs 0.833 s (equal); uk-2005 0.084 vs 0.128 s. Same files.
- **Random access to one list**: 300–369 ns vs 93–113 ns raw (orkut, twitter-sym).
- **Under an 8 GB cap**, twitter-sym, 6 threads, repeated scan: compressed (4.84 GB, stays
  resident) 6,437 M edges/s vs raw (9.95 GB, re-read from disk) 1,232 M edges/s — 5.2×
  (`lab/c-compression/results/dev6_cold8g.jsonl`).

What was **not** measured on the compressed form: complete kernel runs (PageRank to
convergence, WCC, BFS, k-core, triangles), Leiden, point queries and traversals, writing
it during ingest, and any codec number on an idle machine. The kernel numbers in FINDINGS
§ B1/B2/F1 and DESIGN § 2 are raw-array numbers (FINDINGS "What the comparison says": "All
lab kernels ran on uncompressed arrays").

## Result of #118

**Pending.** Experiment #118 (`plan/issues/m1-13-compressed-end-to-end.md`) is measuring
kernels, Leiden and point queries on lj, orkut, twitter-sym and Friendster, idle machine,
1 and 16 threads, in three configurations (raw; compressed with on-the-fly decode;
compressed on disk with a raw copy decoded at open). Its table and the planner rule it
derives are to be recorded here, and the status updated, when it reports. No number from
it is anticipated in this ADR.

## Consequences

- The codec (#30) is built as specified; the planner (#45) gets a decode tier whose rule
  comes from #118.
- Public performance claims (#100) are not made from raw-array numbers; they wait for #118.
- Raw PageRank being 2× faster on orkut is a known cost of the default until #118 says
  how much of it the decode cache recovers.

## What would change our mind

If #118 shows that, on graphs that fit in RAM, the default configuration (compressed on
disk, with whatever decode tier the planner picks within the memory budget) is more than
1.5× slower than raw arrays for PageRank, Leiden or point queries at 16 threads on an idle
machine, then either the in-RAM default becomes raw (compressed stays the on-disk and
out-of-core form) or the public claims are restated — by a new ADR. The 1.5× threshold is
set by this ADR, not measured.

## Closed by

#118 (end-to-end measurement on the default format). Implementation: #30.
