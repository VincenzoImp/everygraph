---
title: I/O layer: ByteSource trait and native backends
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P0, risk:medium, size:L, lab-evidence
---
**Why.** The same kernels must run over memory, mmap, sequential streams, HTTP ranges and browser files (DESIGN D2, D11). The lab also produced specific, non-obvious I/O results that the layer must encode.

**Lab findings to build in**
- mmap-warm equals heap for kernels: mmap can be the only "in-RAM" mode
- O_DIRECT helps for large sequential *input* (Friendster pass 1: 19.0 → 11.1 s) and **hurts** for small appending temp writes (74.6–81.4 s vs 33.2–45.3 s through the page cache)
- 32 threads `pwrite`-ing one output file do not scale; a shared mapping was 10–20% better; an aligned-block writer with preallocation is the proposed fix
- Cold point reads: pinning offsets in RAM removes one of two I/Os; `MADV_WILLNEED` on a whole frontier is essential (8.7 → 0.47 ms)
- Reading a browser `File` reaches 0.9–3 GB/s depending on the API

**Scope**
- [ ] `ByteSource`: `len()`, `read_at(offset, buf)`, optional `as_slice()` for zero-copy backends, `advise(range, hint)`; re-readable by contract
- [ ] Backends: in-memory, mmap, buffered `pread`, O_DIRECT aligned sequential reader with per-worker 8 MiB chunks
- [ ] Sinks: buffered append, aligned-block writer with preallocation, sharded temp logs for spill
- [ ] Segment iterators that hand kernels bounded chunks (prerequisite for {{#m3-01-edge-map}})
- [ ] Windows and macOS equivalents (no `O_DIRECT` on macOS: `F_NOCACHE`)
- [ ] Hooks for later `io_uring` and HTTP-range backends, without implementing them

**Acceptance**
- Sequential scan through the O_DIRECT backend reaches ≥ 80% of the device's sequential bandwidth on the lab NVMe. Met in the official run of 2026-11-25 (`bench/results/io/20261125T205022Z-21498df39590.jsonl`, `bench/specs/io.toml`, idle, medians of three): `direct` 7.10 GB/s at 32 threads against the device reference 7.08 GB/s (`raw`, 4 threads), Friendster `targets.u32` (14.4 GB)
- All backends pass one conformance test-suite
- `everygraph-core` still builds for wasm32 (backends live in `everygraph-io`)
