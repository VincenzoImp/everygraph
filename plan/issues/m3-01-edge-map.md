---
title: Edge-map framework: one kernel body for slices, mmap, streams and compressed blocks
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P0, risk:high, size:XL, lab-evidence
---
**Why.** Without a shared abstraction every kernel would be written three or four times (raw, compressed, streamed, wasm), and the regimes would drift apart. In the lab the PageRank loop body was already identical for in-memory slices and O_DIRECT streams. The browser adds a constraint: a kernel executed as one long call stays in V8's baseline compiler and runs 4.6× slower (lab, PageRank in Chromium), so kernels must be many short calls over chunks.

**Scope**
- [ ] `AdjacencySource` abstraction yielding node-range chunks: (node range, offsets, decoded targets, optional property slices), backed by raw CSR, compressed blocks (decode into a per-thread buffer), mmap, or sequential streams. Scan-shaped drivers decode a batch of consecutive lists (8,192 entries measured) before the kernel body runs, never list by list: list-by-list costs orkut PageRank 1.35× at 1 thread and 1.38× at 16 (#118)
- [ ] `edge_map` / `vertex_map` drivers: pull over all nodes (scan-shaped), push/pull over a frontier (traversal-shaped), with per-thread state and a deterministic reduction order
- [ ] Deterministic chunking: chunk boundaries depend on the graph only, never on the thread count, so floating-point reductions are reproducible ({{#m3-11-determinism-ci}})
- [ ] Views: masks and node filters applied in the driver ({{#m7-04-views}})
- [ ] Random-access path for frontier kernels (list decode by node id) with prefetch hints
- [ ] Budget hooks: per-thread buffers reserved through the accountant ({{#m2-04-arrays-accountant}})
- [ ] No fork-join per BFS level (a weakness noted in the lab kernels)
- [ ] Zero-overhead check: the framework must not cost more than 5% vs the hand-written lab loops
- [ ] A way for tests to run the same kernel at a chosen number of threads inside one process (a scoped pool behind `everygraph_core::par`), so the oracle's cross-configuration checks ({{#m0-05-oracle}}) and the determinism job ({{#m3-11-determinism-ci}}) need no environment variables or subprocesses; rayon stays a dependency of the core only
- [ ] Reductions are performed over fixed node-range blocks owned by the framework, never over the scan chunks, so a kernel's floating-point result does not depend on chunk size, thread count or storage (finding of the oracle, PR #131); the oracle's cross-configuration check runs each kernel at chunk sizes 64 and 65,536
- [ ] The random-access path sorts the frontier and coalesces compressed byte ranges through the Elias–Fano offsets (FlashGraph, Graphene), and scan-shaped kernels skip blocks with no active nodes (GridGraph) (`docs/research/memory-bounded-algorithms.md`)
- [ ] Decoded neighbour lists from a corrupt or hostile file may be unsorted or contain targets ≥ n (the codec checks record framing, not graph invariants — review of PR #157): every kernel driver validates targets (or the graph is validated once at open, with the result cached in the file's provenance) before any unchecked indexing

**Acceptance**
- PageRank, WCC and BFS written once run over all four sources with byte-identical results
- Overhead vs `lab/b1-core` hand-written kernels ≤ 5% on orkut
- orkut PageRank f32 at 16 threads on the compressed source ≤ 1.0× of the raw source (lab: 0.73×)
- The same kernels compile to wasm32 and match native output

**Decision input.** Whether a cache-blocked variant is needed from the start: {{#m1-08-memory-bandwidth}}.

**Depends on** {{#m2-02-io-layer}}, {{#m2-03-adjacency-codec}}

**Left after PR #202 (official run 2026-11-26).** WCC at 16 threads is 1.13× the lab (`bench/results/edge-map/20261126T043042Z-c0d7f248bb47.jsonl`), above the ≤ 5% criterion. Likely a fixed cost per task (about 3,600 tasks per pass on orkut, four lock operations and two shared `fetch_add` each, serial block partials before each pass; read from the code, not profiled, PR #224). Method: hand each worker its reader slot and scratch once per pass (or skip the pool when the reader is `()`), then an interleaved A/B against the lab. Closed on 2026-11-26 on its tests and exploratory evidence (CLAUDE.md § 6); this gap is {{#m3-22-performance-pass}}'s.

