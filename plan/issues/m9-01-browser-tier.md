---
title: Browser tier: drop a file, get a map, nothing installed
milestone: M9 · After v0.1
labels: type:feature, area:wasm, P1, risk:medium, size:L, lab-evidence
---
**Why.** It removes installation for the largest group of users (graphs of 10^5–10^7 edges). The lab says go, with conditions: the same Rust core compiled to wasm32 with threads ran lj (35 M edges) end to end in ~3 s in Chromium against ~1.3 s natively, with identical results.

**Constraints measured in the lab**
- 4 GiB linear memory and no single allocation above 2 GiB: two-pass build fits ~265 M undirected edges; advertise 10–50 M "in seconds", ~100 M as the ceiling
- Kernels must run as many short calls (V8 keeps one-shot kernels in its baseline tier: 4.6× slower in the lab; the "9×" quoted earlier compared with native and mixed two effects); ship only the threaded build; cap the pool around 16
- memory64 is ~3× slower on PageRank and missing in Safari; do not rely on it
- Threads need a pinned nightly with `-Z build-std` and explicit linker flags, and COOP/COEP headers (which restrict embedding)
- OPFS can persist results (684–816 MB/s writes) but there is no mmap and the 4 GiB working set still applies

**Scope**
- [ ] CI job for the threaded wasm32 build (pinned nightly in `tools/nightly-toolchain`, `-Z build-std`, the linker flags of FINDINGS § G), replacing the placeholder comment in `.github/workflows/ci.yml`, with a run under SharedArrayBuffer (moved here from {{#m0-02-ci}})
- [ ] `everygraph-wasm` bindings: ingest from a dropped `File` (two-pass streaming), kernels, communities, layout, in-memory tiles for the viewer
- [ ] Static page (no server): drop a CSV/Parquet → map; export `.eg`/`.egmap`
- [ ] Memory planner for the 4 GiB/2 GiB limits with a clear refusal message
- [ ] Reader path: open a remote `.eg` via HTTP range requests for neighbour queries without downloading it
- [ ] Browser matrix: Chromium, Firefox, Safari (not measured in the lab), mobile
- [ ] Optional WebGPU compute for layout (adapter not available by default in headless Chromium — treat as an accelerator only)

**Acceptance**
- 10 M-edge CSV → interactive map in < 10 s on a mid-range laptop in Chromium and Firefox, entirely client-side
- Results hash-identical to native

**Lab estimate.** Browser shell 2–3 weeks, segmented arrays + planner ~2 weeks (done in M2), CI ~1 week.
