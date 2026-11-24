# Everygraph — product and architecture

Status: design baseline, 2026-11-24. Every decision below cites the measurement that
supports it (see [FINDINGS.md](FINDINGS.md)); decisions without a measurement are marked
**unvalidated**. Numbers are from one workstation (Ryzen 9 7950X, 61 GB RAM, NVMe).

## 1. The problem

People who work with graphs choose between tools that are comfortable but do not scale
(NetworkX, Gephi) and tools that scale but are not comfortable (cluster frameworks,
low-level libraries, server databases). To prepare, analyze, query and look at one graph
they chain three to five tools, converting data at every step, and most of the machine
sits idle. No free tool does all four at any size on one machine.

## 2. The product

**Everygraph: every graph, every size, every machine.** One embeddable engine, one open
file format, one map viewer.

Three promises, each measurable:

1. **Open anything fast.** A file becomes an analysis-ready graph at about 100 M edges/s
   (lab: 35 M edges in 0.4 s, 1.8 B edges in 21 s).
2. **Never run out of memory.** The user gives a memory budget; the engine plans before it
   starts and stays inside it. Out of core, a kernel costs about one sequential read of the
   graph per pass. For iterative kernels that is close to their in-RAM cost (lab: PageRank
   on 1.8 B edges at 3.4 s/iteration in RAM, 3.7 s/iteration under a 2 GB limit); for cheap
   single-pass kernels it dominates (components: 0.34 s in RAM, 3.6 s out of core).
3. **See it and trust it.** A zoomable map of the whole graph with exact numbers at every
   level, bit-identical across runs and thread counts, shareable as one static file.

What it is not: a transactional graph database, a distributed system, a GNN framework, a
hosted service, or a figure editor. It imports from and exports to the tools that are.

### Users, in order of priority for v0.1

| User | Wants | Uses |
|---|---|---|
| Data scientist / researcher, graphs of 10^6–10^9 edges | fast analytics inside Python, no cluster | `pip install everygraph` |
| Analyst / investigator | load a file, understand the structure, share it | CLI + map viewer |
| Researcher at 10^9–10^11 edges | scale, reproducibility, rigor | CLI, server-class machine |
| Developer | embed a graph or a map in an app | Rust crate, map file format, viewer component |
| AI agent | question a huge graph | MCP server (after v0.1) |

### Where we must win, and where we do not try

The 2026 bar: Gephi 0.11 claims 10 M elements; DuckDB, LadybugDB and Grafeo own embedded
querying; NetworKit and GVE own in-RAM kernels. Everygraph wins on the combination and on
four things nobody else offers together: graphs above 10^8 edges on a workstation or
laptop, a hierarchical map down to single nodes, a hard memory budget, and reproducible
results. It does not try to beat Gephi at styling 10^4 nodes or Neo4j at Cypher.

## 3. Architecture

```
 CLI   Python   Rust   Jupyter widget   Map viewer   NetworkX backend   (later: SQL, MCP, WASM page)
  └───────┴───────┴──────────┬─────────────┴─────────────────┘
                     one API: Graph · View · algorithms · queries · map
        ┌────────────────────┴─────────────────────────────────────┐
        │ PLANNER   data + hardware + budget → regime, threads     │
        │           preflight estimate · memory accountant         │
        ├──────────┬────────────┬─────────────┬────────────────────┤
        │ INGEST   │ KERNELS    │ QUERIES     │ MAP                │
        │ parse    │ PageRank   │ neighbors   │ hierarchy shaping  │
        │ dedupe   │ WCC, BFS…  │ k-hop, path │ layout             │
        │ CSR build│ Leiden     │ views       │ tile pyramid       │
        ├──────────┴────────────┴─────────────┴────────────────────┤
        │ EDGE-MAP FRAMEWORK  one kernel body, three sources:      │
        │   in-memory slices · mmap · sequential streams           │
        ├──────────────────────────────────────────────────────────┤
        │ STORE  `.eg` container: compressed topology (out + in),  │
        │   id dictionary, property columns, derived artifacts     │
        │   (hierarchy, layout, indexes) with provenance           │
        ├──────────────────────────────────────────────────────────┤
        │ I/O LAYER  ByteSource trait: file, mmap, O_DIRECT,       │
        │   HTTP range, browser File/OPFS                          │
        └──────────────────────────────────────────────────────────┘
```

Crates: `everygraph-core` (types, memory accountant, parallel facade, ByteSource — no file
system, no mmap, no OS threads assumed), `-io`, `-format`, `-ingest`, `-kernels`,
`-community`, `-map`, `-query`, `-py`, `-cli`, `-wasm`, `-bench`; the viewer is a separate
TypeScript package.

## 4. Decisions

| # | Decision | Evidence | Status |
|---|---|---|---|
| D1 | **Rust core, no C/C++ dependencies in the core.** | Lab kernels are 4–19× faster than NetworKit on orkut (idle machine); the same code runs in the browser. | validated |
| D2 | **Core is portable by construction:** I/O behind a chunked, positional, re-readable `ByteSource`; builders are two-pass; arrays are segmentable (≤ 2 GiB chunks); allocations are fallible and pre-estimated; parallelism behind a facade. | wasm32 has a 4 GiB memory and a 2 GiB single-allocation limit; two-pass build halves peak memory. | validated |
| D3 | **Dense u32 node ids by default**, u64 offsets on disk, a u64-id build for n ≥ 2^32 later. | u64 targets cost +70–85% on PageRank. | validated |
| D4 | **Compressed adjacency is the default storage:** delta + Stream VByte in node-aligned ~64K-edge blocks, Elias–Fano offsets, 4 KiB-aligned sections; own codec (~400 lines). Raw CSR is an in-RAM decode cache the planner may build. | 16–17 bits/edge vs 33 raw; scans faster than raw at 32 threads on twitter; 5.2× faster than raw under an 8 GB cap; raw PageRank 2× faster on orkut. | validated |
| D5 | **webgraph-rs is an optional feature** (BV import/export, LLP ordering, archival tier for web graphs), not the core codec. | BV: 2.1 bits/edge on web graphs but 7–20× slower decode; native deps block WASM; 4 breaking releases in a year. | validated |
| D6 | **Directed graphs store out- and in-adjacency**; the in-adjacency is a derived artifact. | CSC built in 12.3 s for 1.47 B arcs; needed for in-neighbors and directed bidirectional BFS. | validated |
| D7 | **One `.eg` file per graph:** section table, content-hashed sections, derived artifacts appended, `compact` to reclaim space. Maps are exported as a separate standard PMTiles v3 file. | PMTiles v3 with custom payload works and is readable by the official reader. The `.eg` container itself is **unvalidated**. | partly validated |
| D8 | **ID dictionary: typed keys + minimal perfect hash.** Integers → rank bitmap; hex → fixed-width bytes; strings → MPHF (`ptr_hash`) + id-ordered key store; optional front-coded index for prefix search. No FST by default. | 500 M keys built in 11 s, 2.6 bits/key, 0.56 µs lookup, 132 M mappings/s; FST 1.9× raw size on random keys, single-threaded build. | validated |
| D9 | **Properties are Arrow-compatible columns.** Small hot columns (weight, timestamp) duplicated per adjacency entry; wide/cold columns once per edge behind a u32 edge id. | Edge-id indirection makes kernels 1.7–2.1× slower. | validated |
| D10 | **Views: mask or materialize, chosen by the planner.** Bitset mask above ~25% selectivity; materialized filtered CSR below. | Masks never beat the unfiltered scan; materialized view gives PageRank at 0.09× for 1–10% selectivity and costs 1–2 iterations to build. | validated |
| D11 | **Three execution regimes, chosen by the size of the O(n) state, not of the graph:** mmap warm (the only "in-RAM" mode), mmap under pressure, semi-external streams with fixed RSS. A memory accountant backs a no-OOM guarantee. | mmap-warm = heap; Friendster PageRank within ~10% under 8 GB and 2 GB limits; components 10× slower out of core (I/O-bound); the only OOM was O(n) state at 1 GB. | validated for iterative scans; **not** for community detection |
| D12 | **One edge-map framework** so each kernel is written once for all regimes and for WASM (many short calls). | PageRank loop body identical for slices and streams; V8 keeps one-shot kernels in the baseline tier (up to 9× slower). | validated in prototype |
| D13 | **Thread count is chosen per kernel.** Physical cores for memory-bound scans; all hardware threads for compute-bound kernels. | From 16 to 32 threads: no gain for PageRank, components, BFS, k-core; triangles 2.51 → 1.46 s; Louvain on orkut 0.88 → 0.65 s. Friendster PageRank looked flat beyond 8 threads, but only on a loaded machine. | partly validated |
| D14 | **Deterministic by default.** Same input + seed → same output for any thread count and platform. A `fast` mode is opt-in for community detection. | Leiden/Louvain hashes identical at 1–32 threads; layout and tiles byte-identical; cost at 16 threads (idle machine): Leiden 2.4–3.0×, Louvain 1.4–2.0×. | validated; cost above target |
| D15 | **Leiden with block-permuted visiting order and an explicit connectivity guarantee**; 1 iteration = fast, 3 = quality. | Order matters more than Louvain vs Leiden (3–4 modularity points); 3 iterations land within 0.7% of the best reference. | validated on 2 social graphs |
| D16 | **Two community outputs:** the partition (best modularity) and the map hierarchy (fan-out capped, levels compacted). | Natural hierarchies have parents with 100k children; the cap costs 2–5 modularity points. | validated; shaping still weak |
| D17 | **Layout = hierarchy-constrained disc packing**, deterministic. | 3.3 s for 4 M nodes; readable; an embedding layout is a blob. Wastes 97% of the area. | validated; to improve |
| D18 | **Data tiles, not pixels,** in PMTiles v3; dependency-free WebGL2 viewer; static mode and live mode. | First view 152 KB / 18 requests; tiles median 4–6 KB; search in 2 requests. | validated |
| D19 | **Python via PyO3 abi3 + Arrow PyCapsule, bulk columnar API.** | Zero-copy verified; one 525 KB wheel for CPython 3.10–3.14; per-call cost 28–62 ns. | validated |
| D20 | **NetworkX backend is an adoption channel, not the product.** | 1–3× on an existing `nx.Graph` (conversion dominates), 100×+ only with native loading. | validated |
| D21 | **Queries v0.1: traversal API with budgets; no query language.** GQL/Cypher subset later, reusing an existing Rust parser. | Bidirectional BFS 0.3 ms on lj in-process, on raw arrays, vs 6 ms (Neo4j) and 0.3–4.8 s (embedded engines) through their query interfaces — different measurement levels. | validated in prototype |
| D22 | **Browser tier after v0.1:** wasm32 threaded build, ≤ ~100 M edges. | lj end to end in ~3 s in Chromium; ceilings at 265 M edges. | validated |
| D23 | **Licence Apache-2.0 OR MIT; published format specs.** | Kùzu, Memgraph MAGE, CozoDB lessons. | decision |

## 5. What is not solved yet

These are the reasons the first milestone after foundations is a second lab round.

1. **Out-of-core community detection.** Leiden on Friendster needs 23.6 GB of anonymous
   memory; Louvain fits in 4 GB under a 16 GiB limit but returns a poor partition
   (Q 0.465 vs 0.603). The headline "1.8 B edges → map on a 16 GB laptop" has not been
   demonstrated end to end.
2. **Map hierarchy quality.** Top levels are star-shaped; derived regions carry no meaning.
3. **Edges on the map.** Fog inside giant communities; 90% of edge weight undrawn on orkut
   at the deepest zoom.
4. **Cost of determinism for Leiden** (2–3×, stops scaling at ~8 threads).
5. **Memory-bandwidth ceiling** on graphs whose state exceeds the cache.
6. **The pieces were never measured together.** Kernel, community and point-query numbers
   were taken on uncompressed arrays; the compressed format that D4 makes the default was
   measured for scans and PageRank only (PageRank 2× slower on orkut, equal on twitter).
   Ingest timings are from plain text without `fsync` (with it, lj took 0.68 s instead of
   0.37 s in one run).
7. **Evidence is narrow:** two social graphs for community quality; no weighted, directed
   or bipartite community runs; no string-id or Parquet ingest; no Firefox/Safari.
8. **Non-technical:** a possible Uncharted patent on the tile-pyramid pipeline; trademark
   check for the name; a solo maintainer. The short name `eg` is already a package on
   PyPI, crates.io and npm, so the command is `everygraph` until a free alias is chosen.

## 6. Success criteria for v0.1

| Proof | Target |
|---|---|
| 1 M-edge CSV → first map, laptop | < 10 s |
| Friendster ingest, 8 GB budget | < 60 s |
| Friendster PageRank under a 2 GB limit | within 1.35× of in-RAM |
| Friendster communities under a 16 GiB limit | modularity ≥ 0.60, no OOM |
| Same graph at 4 and 32 threads | identical partition, layout and map file |
| Leiden quality (3 iterations) | within 1% of the best reference on 6 graph families |
| Map of a 1 B-edge graph from a static URL | first view < 250 KB; 60 fps on a discrete GPU, usable on integrated |
| Any budgeted run | never OOM-killed |
