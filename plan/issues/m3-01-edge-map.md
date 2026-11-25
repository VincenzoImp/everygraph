---
title: Edge-map framework: one kernel body for slices, mmap, streams and compressed blocks
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P0, risk:high, size:XL, lab-evidence
---
**Why.** Without a shared abstraction every kernel would be written three or four times (raw, compressed, streamed, wasm), and the regimes would drift apart. In the lab the PageRank loop body was already identical for in-memory slices and O_DIRECT streams. The browser adds a constraint: a kernel executed as one long call stays in V8's baseline compiler and runs 4.6× slower (lab, PageRank in Chromium), so kernels must be many short calls over chunks.

**Scope**
- [ ] `AdjacencySource` abstraction yielding node-range chunks: (node range, offsets, decoded targets, optional property slices), backed by raw CSR, compressed blocks (decode into a per-thread buffer), mmap, or sequential streams
- [ ] `edge_map` / `vertex_map` drivers: pull over all nodes (scan-shaped), push/pull over a frontier (traversal-shaped), with per-thread state and a deterministic reduction order
- [ ] Deterministic chunking: chunk boundaries depend on the graph only, never on the thread count, so floating-point reductions are reproducible ({{#m3-11-determinism-ci}})
- [ ] Views: masks and node filters applied in the driver ({{#m7-04-views}})
- [ ] Random-access path for frontier kernels (list decode by node id) with prefetch hints
- [ ] Budget hooks: per-thread buffers reserved through the accountant ({{#m2-04-arrays-accountant}})
- [ ] No fork-join per BFS level (a weakness noted in the lab kernels)
- [ ] Zero-overhead check: the framework must not cost more than 5% vs the hand-written lab loops
- [ ] A way for tests to run the same kernel at a chosen number of threads inside one process (a scoped pool behind `everygraph_core::par`), so the oracle's cross-configuration checks ({{#m0-05-oracle}}) and the determinism job ({{#m3-11-determinism-ci}}) need no environment variables or subprocesses; rayon stays a dependency of the core only

**Acceptance**
- PageRank, WCC and BFS written once run over all four sources with byte-identical results
- Overhead vs `lab/b1-core` hand-written kernels ≤ 5% on orkut
- The same kernels compile to wasm32 and match native output

**Decision input.** Whether a cache-blocked variant is needed from the start: {{#m1-08-memory-bandwidth}}.

**Depends on** {{#m2-02-io-layer}}, {{#m2-03-adjacency-codec}}
