# ADR 0002 — A core portable by construction

- **Status:** accepted
- **Decision:** D2 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

The same core must run as a native library, out of core on a laptop, and in a browser
tab (D22). Each target removes something the others take for granted: the browser has no
file system, no mmap and no OS threads in the usual sense; out-of-core execution must
re-read its input; wasm32 caps memory.

## Decision

I/O goes through a chunked, positional, re-readable `ByteSource`; builders are two-pass;
arrays are segmentable in chunks of at most 2 GiB; allocations are fallible and estimated
before they are made; parallelism sits behind a facade. `everygraph-core` assumes no file
system, no mmap and no OS threads.

## Evidence

All from the browser experiment (FINDINGS § G; shared machine; sizes and pass/fail, not
timings, carry the argument):
- wasm32 has a 4 GiB linear memory and refuses any single allocation above 2 GiB. With the
  two-pass build, Chromium (16 threads) built and ran PageRank and components on a
  generated graph of 265 M undirected edges (529 M entries); two other Chromium runs in
  the same file failed with `RuntimeError: unreachable`, their input size not recorded
  (`lab/g-wasm/results/ceiling.jsonl`). wasm64 in Node went to 300 M edges (5.5 GB).
- Two-pass ingest halves peak memory on lj in Chromium: wasm memory 850 MB with the
  one-pass text build against 438–505 MB with the two-pass build
  (`lab/g-wasm/results/chromium.jsonl`, `wasm_mem_bytes`, modes `text` and
  `text-2pass`). One graph.
- memory64 is not a way out: PageRank was 3.1× slower in wasm64 than in wasm32 in Node
  with the optimizing tier forced (8.15 vs 2.59 s, lj, 1 thread,
  `lab/g-wasm/results/node.jsonl`), and Safari has no memory64 (FINDINGS § G).
- The constraint list itself is FINDINGS § G "Constraints on the core from day one".

What is not measured: the cost of segmented arrays and of the `ByteSource` indirection on
native kernels. The prototypes used flat slices.

## Consequences

- `everygraph-core` has no `std::fs`, no mmap and no thread spawning; the I/O layer (#29)
  and the arrays and memory accountant (#31) implement the contract.
- Every builder that could make one large allocation is written in two passes.
- The edge-map framework (#44) measures the overhead against the lab's flat-slice kernels.

## What would change our mind

- #44 or #31 measure more than 5% overhead on orkut kernels from segmentation or the
  `ByteSource` indirection that cannot be removed for native builds.
- The browser tier (D22) is dropped, which removes the reason for the 2 GiB segment limit.
