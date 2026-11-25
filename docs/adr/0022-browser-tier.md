# ADR 0022 — Browser tier after v0.1

- **Status:** accepted, partly validated
- **Decision:** D22 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

"Every machine" includes a browser tab: drop a file, get a map, nothing installed. The
browser imposes wasm32 limits (ADR 0002) and its own performance traps.

## Decision

A browser tier comes after v0.1: a threaded wasm32 build of the same core, promised for
graphs up to about 100 M edges.

## Evidence

From the browser experiment (FINDINGS § G; shared machine; `lab/g-wasm/results/`):
- **End to end on lj** (drop file → two-pass CSR → PageRank → components): about 3 s in
  Chromium, threaded build (two-pass ingest 2.2–2.8 s plus PageRank 0.62–0.83 s,
  `lab/g-wasm/results/chromium.jsonl`) against about 1.3 s native at 32 threads
  (0.97 s ingest, 0.32 s PageRank, 0.03 s components; `lab/g-wasm/results/native.jsonl`).
- **PageRank, 20 iterations, lj**: native 0.33 s (32 threads); Chromium threaded 0.52 s
  (16 threads), 0.44 s with the optimizing tier forced.
- **Ceiling**: 265 M undirected edges built and analysed in Chromium at 16 threads
  (`lab/g-wasm/results/ceiling.jsonl`); 4 GiB of memory and no allocation above 2 GiB.
  "10–50 M in seconds, ~100 M defensible" is the experiment's judgment from these runs.
- **Traps**: a kernel run as one long call stays in V8's baseline tier (4.6× slower on lj
  in the measured case, ADR 0012); memory64 is ~3× slower and absent from Safari; threads
  need COOP/COEP headers and a pinned nightly toolchain.
- **Not measured**: Firefox, Safari, a laptop (DESIGN § 5, item 7); OPFS only as a spill
  layer (684 MB/s write), with no mmap.
- Results agree with native (ADR 0014).

## Consequences

- The core keeps the portability rules of ADR 0002 from M0, so the browser tier needs a
  shell, not a port.
- #108 builds it after v0.1; its acceptance (10 M-edge CSV → interactive map in < 10 s on a
  mid-range laptop in Chromium and Firefox, hash-identical to native) closes the gaps.

## What would change our mind

- #108 cannot meet its acceptance in Firefox, or the 10 M-edge target needs more than the
  wasm32 limits allow.
- Browsers ship memory64 at wasm32 speed in all three engines, lifting the 100 M ceiling.

## Closed by

#108 (browser tier).
