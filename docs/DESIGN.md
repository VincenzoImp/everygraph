# Everygraph — product and architecture

Status: design baseline, 2026-11-24; D4 amended 2026-11-25 (ADR 0004, #118); promise 1
restated 2026-11-25 with the harness's ingest measurements (ADR 0026); § 5 and § 6 annotated
2026-11-25 with the tracer bullet (#15). Every decision below cites the measurement that
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
   while what the ingest writes fits in memory or in the drive's fast-write burst. A graph
   whose ingest writes more is bounded by the drive's sustained write rate, not by the
   CPU: about 60 M edges/s for Friendster on the lab machine (ADR 0026). Measured with the
   lab prototype through the benchmark harness (idle machine, text to raw arrays, 32
   threads, no `fsync`; `bench/results/headline/`, 2026-11-25): lj 35 M edges in 0.386 s
   and orkut 117 M in 0.781 s (90 M and 150 M edges/s, median of 6); Friendster 1.8 B
   edges in 29.06 s with a 32 GB budget and 36.97 s with 8 GB when ingests run back to back
   (62 M and 49 M edges/s, median of 3 after a warm-up), 17.7–19.5 s on a rested drive
   (92–102 M edges/s). Encoding to the default compressed storage is a further 0.06 s,
   0.17 s and 3.4 s at 16 threads, measured separately.
2. **Never run out of memory.** The user gives a memory budget; the engine plans before it
   starts and stays inside it. Out of core, a kernel costs about one sequential read of the
   graph per pass. For iterative kernels that is close to their in-RAM cost (lab, Friendster,
   1.8 B edges, PageRank f32, idle: 3.2 s/iteration in RAM on the default compressed storage
   at 16 threads, 3.4 on raw arrays at 16 and 32 threads; 3.6 s/iteration under a 2 GB limit
   with the raw-format streaming reader at 32 threads — on the default storage, through plain
   mmap and without a streaming reader, 4.3 s/iteration at 16 threads); for cheap
   single-pass kernels it dominates (components: 0.50 s in RAM on the default storage at
   16 threads, 0.34–0.36 s on raw arrays at 16–32 threads; 3.6 s out of core, measured on
   raw arrays only).
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
| [D1](adr/0001-rust-core.md) | **Rust core, no C/C++ dependencies in the core.** | Lab kernels are 4.4–19× faster than NetworKit on orkut (idle machine; lab at 8–16 threads, NetworKit at its faster 32; uncompressed arrays); the same code runs in the browser. | accepted |
| [D2](adr/0002-portable-core.md) | **Core is portable by construction:** I/O behind a chunked, positional, re-readable `ByteSource`; builders are two-pass; arrays are segmentable (≤ 2 GiB chunks); allocations are fallible and pre-estimated; parallelism behind a facade. | wasm32 has a 4 GiB memory and a 2 GiB single-allocation limit; two-pass build halves peak memory (lj in Chromium: 850 MB → 438–505 MB). | accepted |
| [D3](adr/0003-dense-u32-ids.md) | **Dense u32 node ids by default**, u64 offsets on disk, a u64-id build for n ≥ 2^32 later. | u64 targets cost +71% (f64) to +87% (f32) on PageRank (orkut, 32 threads, idle). | accepted |
| [D4](adr/0004-compressed-adjacency-default.md) | **Compressed adjacency is the default storage:** per-node delta + Stream VByte records in node-aligned ~64K-edge blocks, 4 KiB-aligned sections; own codec (~400 lines). Conditions (amended 2026-11-25 by #118): per-node Elias–Fano byte offsets (not block offsets only); an Elias–Fano index of entry offsets in the format; sequential access decodes batches of lists in the edge-map framework; budgeted queries decode list prefixes; raw CSR is an in-RAM decode cache that the planner builds only for sessions of repeated traversals or point lookups that fit the budget and repay the decode. | End to end on an idle machine (#118; lj, orkut, twitter-sym, Friendster; same binary, raw vs compressed, graphs in RAM): 16.1–18.1 bits/edge, Friendster 23.2 (raw 32.8–35.7); compressed ÷ raw time at 16 threads PageRank 0.73–0.97, Leiden (one iteration, async and deterministic) 0.99–1.04, WCC 0.88–1.43, BFS 1.20–1.52 (1 thread: BFS 1.78–2.04, PageRank on lj 1.24–1.40); neighbour lookup +0.02–0.11 µs, 1 thread; under a cap the raw arrays exceed, PageRank 1.2–1.9× and Friendster WCC/BFS 5.4–8.8× faster compressed. The first lab's "raw PageRank 2× faster on orkut" (shared machine) is not reproduced. Not validated: web graphs, weighted or directed kernels, ingest writing the format, out-of-core streaming on it, point-query throughput at 16 threads. | accepted, partly validated (#46, #35, #24, #90) |
| [D5](adr/0005-webgraph-optional.md) | **webgraph-rs is an optional feature** (BV import/export, LLP ordering, archival tier for web graphs), not the core codec. | BV: 2.1 bits/edge on web graphs but 5.0–21.9× slower sequential decode (9.1–21.9× on social graphs; 1 and 32 threads, shared machine); native deps make WASM unlikely (judged, not tried); 4 breaking releases in a year. | accepted |
| [D6](adr/0006-in-adjacency.md) | **Directed graphs store out- and in-adjacency**; the in-adjacency is a derived artifact. | CSC built in 12.3 s for 1.47 B arcs (twitter-2010, 32 threads, shared machine); needed for in-neighbors and directed bidirectional BFS. | accepted |
| [D7](adr/0007-eg-container.md) | **One `.eg` file per graph, in every state** (builds in progress write `NAME.eg.partial` and keep intermediates in the job's scratch; [ADR 0027](adr/0027-single-file-builds.md)): a table of contents behind two alternating header blocks, BLAKE3-hashed sections, derived artifacts appended and committed by a crash-safe protocol, `compact` to reclaim space (`docs/spec/eg-container.md`). Maps are exported as a separate standard PMTiles v3 file. | PMTiles v3 with custom payload works and is readable by the official reader. Container (#28, tests, no timing): opening costs two reads on a synthetic 15 GB file; a crash at every write step, under every image the sync model allows, and a real `kill -9` leave the previous or the new commit with every section the file held unchanged; an independent Python reader lists and verifies a file written by Rust. | accepted |
| [D8](adr/0008-id-dictionary.md) | **ID dictionary: typed keys + minimal perfect hash.** Integers → rank bitmap; hex → fixed-width bytes; strings → MPHF (`ptr_hash`) + id-ordered key store; optional front-coded index for prefix search. No FST by default. | MPHF over 500 M keys built in 10.8 s at 32 threads (whole dictionary with key store 19.4 s), 2.6 bits/key, 0.56 µs lookup; batch mapping 132 M/s in one unverified run; FST 1.9× raw size on random binary keys (0.28× on URL-like keys), single-threaded build. Shared machine. | accepted, partly validated (#34, #25) |
| [D9](adr/0009-property-columns.md) | **Properties are Arrow-compatible columns.** Small hot columns (weight, timestamp) duplicated per adjacency entry; wide/cold columns once per edge behind a u32 edge id. | Edge-id indirection makes weighted PageRank 1.7× (4 threads) to 2.1× (32 threads) slower — one kernel, orkut, shared machine. | accepted, partly validated (#37) |
| [D10](adr/0010-views-mask-or-materialize.md) | **Views: mask or materialize, chosen by the planner.** A bitset mask or a materialized filtered CSR; the planner's rule is derived from a measured crossover by #93 (the earlier ~25% selectivity threshold was not a measurement). | orkut, shared machine: a bitset mask never clearly beats the unfiltered scan (0.91–2.09× per PageRank iteration); a materialized view runs PageRank at 0.09× (1%) and 0.09–0.17× (10%; 4 / 32 threads) and costs about 1–2.5 unfiltered iterations to build. It was faster per iteration than the mask at every selectivity measured (1–90%); the mask saves build time and memory. | open — needs experiment (#93) |
| [D11](adr/0011-execution-regimes.md) | **Three execution regimes, chosen by the size of the O(n) state, not of the graph:** mmap warm (the only "in-RAM" mode), mmap under pressure, semi-external streams with fixed RSS. A memory accountant backs a no-OOM guarantee. | mmap-warm = heap; Friendster PageRank f32 within ~10% under 8 GB and 2 GB limits (idle, 32 threads); components 10× slower out of core (I/O-bound); the only OOM was O(n) state at 1 GB. For Leiden the dominant memory is the level-1 quotient graph, not the O(n) state; with it spilled, one Leiden iteration on Friendster runs under 16 GiB at 1.17× and under 8 GiB at 2.34× the same algorithm in RAM (lab, #16, idle, 16 threads). | accepted, partly validated: iterative scans yes; community detection in the lab, one case (#16), production #60 |
| [D12](adr/0012-edge-map-framework.md) | **One edge-map framework** so each kernel is written once for all regimes and for WASM (many short calls). | PageRank loop body identical for slices and streams (copied in the prototype, not shared); V8 keeps a one-shot kernel in the baseline tier (4.6× slower: lj, 1 thread, Chromium, shared machine). | accepted, partly validated (#44) |
| [D13](adr/0013-threads-per-kernel.md) | **Thread count is chosen per kernel.** Physical cores for memory-bound scans; all hardware threads for compute-bound kernels. | From 16 to 32 threads (idle): no gain for PageRank, components, BFS, k-core on orkut; triangles 2.51 → 1.46 s; async Louvain on orkut 0.88 → 0.65 s. Not general: async Leiden on lj 0.76 → 0.78 s, and every deterministic community run is slower at 32 (lj and orkut). Friendster PageRank looked flat beyond 8 threads, but only on a loaded machine. | accepted, partly validated (#22) |
| [D14](adr/0014-deterministic-by-default.md) | **Deterministic by default.** Same input + seed → same output for any thread count, any platform, any memory budget and whichever way the planner executes it (in memory or out of core; a view as a mask or materialized). The planner never substitutes a lower-precision variant by itself. Not covered: results cut short by `timeout`, and the opt-in `fast` mode for community detection. Widened on 2026-11-25 (`design/DECISIONS.md`, "What 'repeatable' promises"); narrowed only by ADR. | Leiden/Louvain hashes identical at 1–32 threads (lj, orkut); layout and tiles byte-identical; PageRank and components agree across native and wasm; identity across budgets and regimes not yet measured (tests: #54, #45, #47, #48, #93; the case that could narrow it: out-of-core Leiden, #60); cost at 16 threads (idle machine): Leiden 2.4–3.0×, Louvain 1.4–2.0×. | accepted, partly validated (#18, #54, #60); cost above target |
| [D15](adr/0015-leiden-visiting-order.md) | **Leiden with block-permuted visiting order and an explicit connectivity guarantee**; 1 iteration = fast, 3 = quality. | Order matters more than Louvain vs Leiden: id order costs 2.0–2.1 modularity points at one iteration (idle, 16 threads; 2.6–4.2 in earlier shared runs), the two algorithms differ by ≤ 0.4; block order takes 1.38–1.63× the time; 3 iterations land within 0.74% of the best reference (lj, orkut). | accepted, partly validated: 2 social graphs (#24, #57) |
| [D16](adr/0016-two-community-outputs.md) | **Two community outputs:** the partition (best modularity) and the map hierarchy (fan-out capped, levels compacted). | Natural hierarchies have parents with up to 102,821 (lj) and 338,769 (orkut) children; the cap costs 2.3–5.0 modularity points (idle, 16 threads). | accepted, partly validated; shaping still weak (#19) |
| [D17](adr/0017-layout-disc-packing.md) | **Layout = hierarchy-constrained disc packing**, deterministic. | 3.3 s for 4 M nodes (lj, 32 threads, shared machine); median edge length ÷ random-pair distance 0.014 vs 0.244 for an embedding layout (a blob); node discs cover about 1–3% of the map (estimate, not measured). | accepted, partly validated (#21) |
| [D18](adr/0018-data-tiles.md) | **Data tiles, not pixels,** in PMTiles v3; dependency-free WebGL2 viewer; static mode and live mode. | lj, sizes: first view 152 KB / 18 requests; tiles median 4–6 KB; search in 2 requests. Not measured: live mode, integrated GPUs, Firefox, Safari. Above 117 M edges, only the Friendster tracer (#15, idle, RTX 4090): first view 163,184 B / 18 requests, 6.84 GB file, median tile 6.8 KB. | accepted, partly validated (#26, #15) |
| [D19](adr/0019-python-pyo3-arrow.md) | **Python via PyO3 abi3 + Arrow PyCapsule, bulk columnar API.** | Zero-copy verified; one abi3 wheel for CPython 3.10–3.14 (size 525 KB: estimate, not in a result file); per-call cost 28–40 ns (lj, shared machine). | accepted |
| [D20](adr/0020-networkx-backend.md) | **NetworkX backend is an adoption channel, not the product.** | lj, 3 threads, shared machine: 0.92–2.0× on an existing `nx.Graph` including conversion (conversion dominates); with native loading, kernel time only, PageRank 15.6–70× and components 24–53×. | accepted |
| [D21](adr/0021-queries-traversal-api.md) | **Queries v0.1: traversal API with budgets; no query language.** GQL/Cypher subset later, reusing an existing Rust parser. | Bidirectional BFS 0.3 ms on lj in-process, on raw arrays, single thread, shared machine (idle, #118: 0.13 ms raw, 0.16 ms on the compressed default, p50), vs 6.4 ms (Neo4j) and 0.3–4.8 s (embedded engines) through their query interfaces, 3 threads, shared machine — different measurement levels. | accepted, partly validated (#91) |
| [D22](adr/0022-browser-tier.md) | **Browser tier after v0.1:** wasm32 threaded build, ≤ ~100 M edges. | lj end to end in ~3 s in Chromium (threaded build, shared machine); ceiling at 265 M edges; Chromium and Node only. | accepted, partly validated (#108) |
| [D23](adr/0023-licence-and-open-specs.md) | **Licence Apache-2.0 OR MIT; published format specs.** | Kùzu, Memgraph MAGE, CozoDB lessons. | accepted |
| [D24](adr/0024-visual-system.md) | **One visual system ("Vernier") for viewer, docs, notebook, terminal and figures**: warm paper and soft black, one ink-blue accent, every number in monospace, the two-ring mark; values in `design/preview/style/assets/tokens.css`, rules in `design/preview/style/SPEC.md`. | Three rounds of previews; data palette and text contrast measured (worst categorical pair ΔE 15 normal vision, ~7.3 colour-blind; all text tokens ≥ 4.5:1). | accepted (approved by the owner, 2026-11-25) |
| [D25](adr/0025-user-facing-choices.md) | **User-facing choices are fixed in `design/DECISIONS.md`**: API shape and names, CLI messages, graph-semantics defaults, map style (bubbles, dark canvas, links for the selection only, five reused region colours, artefact regions drawn as such). | Owner approved the previews in `design/preview/` as a whole. | accepted; each can be reopened when built |

## 4b. How it looks and reads

The design is not described here but shown: `design/preview/README.md` indexes the
previews (viewer, API, command line, choices), `design/preview/style/` holds the full
style guide, and `design/DECISIONS.md` records what was chosen. Issues that build
something a user sees or types carry a "Design reference" paragraph pointing to the
exact page.

## 5. What is not solved yet

These are the reasons the first milestone after foundations is a second lab round.

1. **Out-of-core community detection: solved in the lab for one case, not yet in the
   product** (rewritten 2026-11-25 from #16, FINDINGS § M1-02; idle machine;
   `bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl` and
   `…20261125T174959Z-b61036e59740.jsonl`). Case measured: one iteration of Leiden, async
   and deterministic, on Friendster, 16 threads, f64 weights, level 0 in the D4 compressed
   store, against the same algorithm in RAM (74.8 s). The earlier 22.6 GiB need was that
   of member-order aggregation, which holds the level-1 graph in memory. With sequential
   aggregation into budgeted buckets and every quotient graph spilled as a compressed
   store:
   - under 16 GiB: 87.4 s, 1.17×, 0 disconnected communities, async Q median 0.6014 (one
     of two repetitions 0.5981, inside the run-to-run spread of 0.598–0.607); the
     deterministic mode gives the in-RAM partition (same hash, Q 0.6055);
   - under 8 GiB: 175.3 s, 2.34× — the smallest budget within 2.5×; under 4 GiB it still
     completes, at 3.49×. Below 16 GiB the cost is level-0 local moving re-reading the
     store 6–13× per sweep in ADR 0015's block-permuted order;
   - map-shaped Leiden, which the tracer (#15) could not run even at 56 GiB, completes under
     16 GiB: 164.1 s, Q 0.5900, 27 top communities.
   What remains: the production implementation (#60); more than one iteration; the
   deterministic mode below 16 GiB; other graph families (only Friendster at this scale,
   orkut scaled down at 3.10×); and whether a seeded relabelling at ingest, which kept
   Friendster at 1.28–1.43× under 8 and 4 GiB, should replace block-permuted order
   (#146, `plan/issues/m1-14-relabel-at-ingest.md`; ADR 0015 unchanged).
   The headline "1.8 B edges → map on a 16 GB laptop" has not been demonstrated end to
   end. *Measured end to end on 2026-11-25 (#15, FINDINGS § M1-01; Friendster, lab
   binaries, 32 threads, idle, uncompressed CSR,
   `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`): the chain does not
   complete under 16 GiB.* Three blockers, each with its owner:
   - *Community detection with a map-shaped hierarchy* — addressed in the lab by #16
     (above). In the tracer, Leiden and fan-out-capped Louvain were killed in the first
     aggregation while thrashing on the mapped graph (1.8–2.7 TB read), and the prototype
     kept every level's quotient graph in memory. Plain Louvain fits (4.2 GiB, Q 0.6038;
     the § B2 Q 0.465 was not reproduced, #17) but has a parent with 18.7 M children, which
     the layout cannot use even with 56 GiB. Owners now: #60 (out-of-core Leiden in the
     product), #19 and #63 (shaping bounded in memory), #62 (levels streamed to disk).
   - *The layout loads the graph into the heap.* Killed under 16 GiB that way; with the
     graph memory-mapped it fits (3.7 GiB anonymous) at 5.8× the time, re-reading 733 GB.
     Owner: #67.
   - *The tile builder keeps every tile in RAM*, and each zoom level's working state:
     29.1 GiB anonymous with 56 GiB; under 16 GiB (13.9 GiB anonymous, one pass over the
     graph per zoom level), cut by the 60-minute timeout before the last zoom level.
     Owner: #69.
2. **Map hierarchy quality.** Top levels are star-shaped; derived regions carry no meaning.
3. **Edges on the map.** Fog inside giant communities; 90% of edge weight undrawn on orkut
   at the deepest zoom.
4. **Cost of determinism for Leiden** (2.4–3.0× at 16 threads on orkut and lj, idle
   machine; stops scaling at ~8 threads).
5. **Memory-bandwidth ceiling** on graphs whose state exceeds the cache.
6. **The pieces measured together — in RAM only.** *Answered for analytics in RAM on
   2026-11-25 (#118, FINDINGS § M1-13):* kernels, Leiden and single-thread point queries
   now have idle numbers on the default compressed storage, against raw arrays in the same
   binary, on four social graphs; the default holds with the conditions written into D4.
   Still not measured on the default storage: ingest writing it (timings are from plain
   text to raw arrays without `fsync`; with it, lj took 0.68 s instead of 0.37 s in one
   run) and an encoder under a budget; a streaming reader out of core; point-query
   throughput at 16 threads; web graphs; weighted and directed kernels.
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

Measured against two of these targets on the default compressed storage (#118, 2026-11-25,
idle machine, FINDINGS § M1-13) and, for the ingest, with the benchmark harness (#4,
2026-11-25, idle machine, FINDINGS "Idle-machine re-runs"); the targets are unchanged:

- *Friendster PageRank under a 2 GB limit, within 1.35× of in-RAM:* through plain mmap of
  the compressed store, 16 threads, 4.31 s/iteration against 3.19 in RAM — 1.35×, no margin
  (arithmetic on a one-repetition, 5-iteration capped run that includes the initial degree
  pass; raw arrays the same way: 1.32×). The lab's raw-format streaming reader gave 1.08×
  (3.64 vs 3.37 s/iteration, 32 threads). The gap: no streaming reader for the compressed
  format exists yet (#46).
- *Friendster ingest, 8 GB budget, under 60 s:* the ingest itself to raw arrays took
  36.97 s (35.06–37.14, median of 3) at the drive's sustained write rate (benchmark
  harness, idle machine, 32 threads, no `fsync`, one untimed warm-up repetition first;
  `bench/results/headline/20261125T114658Z-0ffe756ac9ac.jsonl`, rows 4–6; ADR 0026). The
  lab's single run, 52.4 s, is superseded. Encoding to the default format took 3.41 s at
  16 threads plus 4.77 s to write it with `fsync`, measured separately (#118), so
  36.97 + 3.41 = 40.4 s by addition before the write and 45.2 s with it. What remains
  unmeasured is the part that decides the target: the encoder measured holds input and
  output in RAM (about 14 GiB raw and 9.8 GiB compressed on Friendster), so it cannot run
  under 8 GB as it is. The gap: an encoder that works under the budget (#35, ADR 0004),
  and the build writing the default format directly, which changes the bytes written and
  so, on this drive, the time (ADR 0026).

Measured against the 16 GiB criteria by the tracer bullet (#15, 2026-11-25, idle machine,
lab binaries, Friendster, 32 threads, uncompressed CSR, FINDINGS § M1-01,
`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`); the targets are unchanged:

- *Friendster communities under a 16 GiB limit, modularity ≥ 0.60, no OOM:* met only by
  plain Louvain, whose hierarchy the map cannot use: 115.1 s, 4.2 GiB anonymous, Q 0.6038,
  62 disconnected communities, a parent with 18.7 M children (row 15). Under 16 GiB every
  map-shaped community run was killed: Leiden at 460.5 s, fan-out-capped Louvain at 648 s
  (rows 9, 11–12). With 56 GiB and hierarchy output the capped Louvain completes (42.9 GiB
  anonymous peak, Q 0.5757) and the capped Leiden is killed. Blockers and owners:
  out-of-core aggregation (#16, #60), hierarchy shaping bounded in memory (#19, #63), the
  hierarchy's quotient graphs kept in memory (#62).
  *Communities part measured feasible, 2026-11-25 (#16, FINDINGS § M1-02, idle, lab code,
  Friendster, 1 iteration, 16 threads, compressed level 0):* Leiden under 16 GiB completes
  with 0 disconnected communities at 1.17× the same algorithm in RAM; Q 0.6055 in the
  deterministic mode, async median 0.6014 with one repetition at 0.5981; map-shaped Leiden
  completes under 16 GiB (Q 0.5900). Not yet in the product (#60); the layout (#67) and
  the tile builder (#69) remain. The target is unchanged.
- *The headline "1.8 B edges → map on a 16 GB laptop"* (§ 5, item 1) also needs the
  layout never to load the graph into the heap (#67: killed in the heap; memory-mapped it
  fits, at 5.8× the time, default readahead) and a tile builder that bounds what it holds
  in RAM, tiles and per-zoom working state (#69: not finished in 60 minutes under 16 GiB).
  With 56 GiB the chain completes in 455.5 s of stage time (sum of medians), with the
  capped Louvain hierarchy.
- *Map of a 1 B-edge graph from a static URL, first view < 250 KB:* 163,184 B in 18
  requests on the Friendster map (RTX 4090, headless Chromium, row 31). Frames 16.67 ms
  mean, none over 33 ms: vsync-capped, so 60 fps held on that GPU but the headroom is
  unknown; integrated GPUs not measured (#26).
