# Everygraph — product and architecture

Every decision below cites the measurement that supports it (see [FINDINGS.md](FINDINGS.md));
a decision without a measurement is marked **unvalidated**. The record of each decision,
with its amendments, is its ADR under `docs/adr/`. Numbers are from one workstation (Ryzen 9
7950X, 61 GB RAM, NVMe). A number labelled "exploratory" or "shared machine" was taken
while other work ran; the others are from an idle machine.

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

Most of the parts are prior art (checked 2026-11-25, `docs/research/README.md` A1):

- a hierarchical map served as data tiles: Uncharted / Aperture Tiles, Langevin et al. 2015;
- a fan-out cap of 64: ASK-GraphView 2006;
- compressed adjacency faster than raw in parallel: Ligra+ 2015;
- recursive sub-clustering of oversized communities: ASK-GraphView, hierarchical Infomap,
  graspologic;
- Elias–Fano offsets: webgraph-rs;
- seeded relabelling of the node order: DSLM 2018;
- planning and refusing before a run: Neo4j GDS memory estimation.

What we can claim, because no work we read does it:

- identical results across thread count, memory budget, execution regime and view
  representation — new for graph engines as far as read; the concept is not new
  elsewhere: decomposition-invariant sums in HPC (Hallberg–Adcroft 2014) and data
  independence of floating-point aggregation in databases (Müller et al. 2018;
  `docs/research/sota-numerics.md` § 5, `docs/research/README.md` N7);
- a deterministic, budgeted text → CSR build;
- a map of a 1.8 B-edge graph on one machine, under a budget, byte-identical, with declared
  omissions (the aim; not yet demonstrated end to end, § 5 item 1);
- one seeded permutation for storage, layout and visiting order. The ordering itself is
  not new (Rabbit Order, LLP); reusing the product's own hierarchy for all three is.

Two more claims hold only in part:

- the planner's raw decode-cache tier with its break-even rule (probably new as a planner
  policy; the reasoning is standard);
- a planned, typed refusal before work starts. Neo4j GDS also refuses before a run;
  DuckDB and Velox fail mid-run.

The map's super-level rule is our heuristic; no prior art was found for it.

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
`-community`, `-map`, `-query`, `-py`, `-cli`, `-wasm`, `-bench`, and `-oracle` (the
independent correctness oracle, a test tool, not product code); the viewer is a separate
TypeScript package.

## 4. Decisions

| # | Decision | Evidence | Status |
|---|---|---|---|
| [D1](adr/0001-rust-core.md) | **Rust core, no C/C++ dependencies in the core.** | Lab kernels are 4.4–19× faster than NetworKit on orkut (idle machine; lab at 8–16 threads, NetworKit at its faster 32; uncompressed arrays); the same code runs in the browser. | accepted |
| [D2](adr/0002-portable-core.md) | **Core is portable by construction:** I/O behind a chunked, positional, re-readable `ByteSource`; builders are two-pass; arrays are segmentable (≤ 2 GiB chunks); allocations are fallible and pre-estimated; parallelism behind a facade. | wasm32 has a 4 GiB memory and a 2 GiB single-allocation limit; two-pass build halves peak memory (lj in Chromium: 850 MB → 438–505 MB). | accepted |
| [D3](adr/0003-dense-u32-ids.md) | **Dense u32 node ids by default**, u64 offsets on disk, a u64-id build for n ≥ 2^32 later. | u64 targets cost +71% (f64) to +87% (f32) on PageRank (orkut, 32 threads, idle). | accepted |
| [D4](adr/0004-compressed-adjacency-default.md) | **Compressed adjacency is the default storage:** per-node delta + Stream VByte records in node-aligned ~64K-edge blocks, 4 KiB-aligned sections; own codec (~400 lines). Conditions (amended 2026-11-25 by #118): per-node Elias–Fano byte offsets (not block offsets only); an Elias–Fano index of entry offsets in the format; sequential access decodes batches of lists in the edge-map framework; budgeted queries decode list prefixes; raw CSR is an in-RAM decode cache that the planner builds only for sessions of repeated traversals or point lookups that fit the budget and repay the decode. | End to end on an idle machine (#118; lj, orkut, twitter-sym, Friendster; same binary, raw vs compressed, graphs in RAM): 16.1–18.1 bits/edge, Friendster 23.2 (raw 32.8–35.7); compressed ÷ raw time at 16 threads PageRank 0.73–0.97, Leiden (one iteration, async and deterministic) 0.99–1.04, WCC 0.88–1.43, BFS 1.20–1.52 (1 thread: BFS 1.78–2.04, PageRank on lj 1.24–1.40); neighbour lookup +0.02–0.11 µs, 1 thread; under a cap the raw arrays exceed, PageRank 1.2–1.9× and Friendster WCC/BFS 5.4–8.8× faster compressed. The first lab's "raw PageRank 2× faster on orkut" (shared machine) is not reproduced. Not validated: web graphs, weighted or directed kernels, the speed of ingest writing the format (the build of #35 writes it since PR #215; tests, and its speed is #240), out-of-core streaming on it, point-query throughput at 16 threads. | accepted, partly validated (#46, #240, #24, #90) |
| [D5](adr/0005-webgraph-optional.md) | **webgraph-rs is an optional feature** (BV import/export, LLP ordering, archival tier for web graphs), not the core codec. | BV: 2.1 bits/edge on web graphs but 5.0–21.9× slower sequential decode (9.1–21.9× on social graphs; 1 and 32 threads, shared machine); native deps make WASM unlikely (judged, not tried); 4 breaking releases in a year. | accepted |
| [D6](adr/0006-in-adjacency.md) | **Directed graphs store out- and in-adjacency**; the in-adjacency is a derived artifact. | CSC built in 12.3 s for 1.47 B arcs (twitter-2010, 32 threads, shared machine); needed for in-neighbors and directed bidirectional BFS. | accepted |
| [D7](adr/0007-eg-container.md) | **One `.eg` file per graph, in every state** (builds in progress write `NAME.eg.partial` and keep intermediates in the job's scratch; [ADR 0027](adr/0027-single-file-builds.md)): a table of contents behind two alternating header blocks, BLAKE3-hashed sections, derived artifacts appended and committed by a crash-safe protocol, `compact` to reclaim space (`docs/spec/eg-container.md`). Maps are exported as a separate standard PMTiles v3 file. | PMTiles v3 with custom payload works and is readable by the official reader. Container (#28, tests, no timing): opening costs two reads on a synthetic 15 GB file; a crash at every write step, under every image the sync model allows (argued, not measured on hardware), leaves the previous or the new commit with every section the file held unchanged, and so does a real `kill -9` (process death only); an independent Python reader lists and verifies a file written by Rust. Results kept in the file and found by the graph's identity and a cache key (#39), maps by range (#204) and one writer at a time across processes (#205): tests, no timing. | accepted |
| [D8](adr/0008-id-dictionary.md) | **ID dictionary: typed keys + minimal perfect hash.** Dense integers in sorted order (all in `0..2^32`, largest + 1 ≤ 64 × count) → rank bitmap; hex, UUID, base58 and fixed-size binary keys → fixed-width bytes; other keys → MPHF (the `PtrHash` construction, implemented in the workspace: the `ptr_hash` crate's output differs from run to run and it fails `cargo deny`; amended 2026-11-25 by #34) + id-ordered key store; optional front-coded index for prefix search. No FST by default. | With the replaced `ptr_hash` crate (lab, shared machine): MPHF over 500 M keys built in 10.8 s at 32 threads (whole dictionary with key store 19.4 s), 2.6 bits/key, 0.56 µs lookup; batch mapping 132 M/s in one unverified run — these belong to the crate. The implementation that replaced it, official run (16 threads, idle; `bench/results/id-dictionary/20261125T220117Z-c855fdd77cf9.jsonl`): 65 M hex keys built in 9.3–10.0 s at 2.61 bits/key, 3.6 B mappings in 34–38 s; 500 M 20-byte keys built under a 4 GiB budget and `MemoryMax` 5 GiB in 76.4 s, 24.3 bytes/key, anonymous peak 3.5 GiB, no OOM event (`bench/results/id-dictionary/20261126T042659Z-c0d7f248bb47.jsonl`). FST 1.9× raw size on random binary keys (0.28× on URL-like keys), single-threaded build. | accepted, partly validated (#25) |
| [D9](adr/0009-property-columns.md) | **Properties are Arrow-compatible columns.** The adjacency stores pairs; a pair's events are found through the adjacency's `.events` offsets ([ADR 0028](adr/0028-pairs-and-events.md), amended 2026-11-25). Small hot columns aligned with each adjacency (duplicated across its ends): per event (timestamp, each row's weight) by event offset, per pair (the pair aggregates kernels read) by entry offset; wide/cold columns once per edge behind an edge id (itself a per-event column; u64 from 2^32 events). | Edge-id indirection makes weighted PageRank 1.7× (4 threads) to 2.1× (32 threads) slower — one kernel, orkut, shared machine. Column sections (#37, `docs/spec/eg-columns.md`): pyarrow and Polars read them without Everygraph, every buffer in place at a multiple of 4096 (`tools/check_columns_python.py`, in CI); tests, no timing. Not yet: the build carrying columns (`plan/issues/m2-34-build-columns.md`); weighted PageRank on a per-pair column against the unweighted kernel, measured (#240). | accepted, partly validated (#240, `m2-34-build-columns`) |
| [D10](adr/0010-views-mask-or-materialize.md) | **Views: mask or materialize, chosen by the planner.** A bitset mask or a materialized filtered CSR; the planner's rule is derived from a measured crossover by #93 (the earlier ~25% selectivity threshold was not a measurement). | orkut, shared machine: a bitset mask never clearly beats the unfiltered scan (0.91–2.09× per PageRank iteration); a materialized view runs PageRank at 0.09× (1%) and 0.09–0.17× (10%; 4 / 32 threads) and costs about 1–2.5 unfiltered iterations to build. It was faster per iteration than the mask at every selectivity measured (1–90%); the mask saves build time and memory. | open — needs experiment (#93) |
| [D11](adr/0011-execution-regimes.md) | **Three execution regimes, chosen by the size of the O(n) state, not of the graph:** mmap warm (the only "in-RAM" mode), mmap under pressure, semi-external streams with fixed RSS. A memory accountant backs a no-OOM guarantee. Regime 2 for random access whose working set exceeds the budget is not plain mmap. The explicit budgeted cache over `ByteSource` is built (`BlockCache`, read ahead by a pool of `pread` threads in the visiting order, #178); it replaces per-phase `MADV_RANDOM` only where a measured comparison at equal memory shows it reads fewer bytes. Elsewhere such a phase uses `MADV_RANDOM`, set per phase and never when the working set fits ([ADR 0029](adr/0029-random-access-under-a-budget.md), amended 2026-11-26). | mmap-warm = heap; Friendster PageRank f32 within ~10% under 8 GB and 2 GB limits (idle, 32 threads); components 10× slower out of core (I/O-bound); the only OOM was O(n) state at 1 GB. For Leiden the dominant memory is the level-1 quotient graph, not the O(n) state; with it spilled, one Leiden iteration on Friendster runs under 16 GiB at 1.17× and under 8 GiB at 2.34× the same algorithm in RAM (lab, #16, idle, 16 threads). Random access under a budget (#172, Friendster, 16 threads, shared machine; bytes read are load-independent, times exploratory): plain mmap read 37–42× the CSR per layout run under 16G and 36–90× the store for Leiden level 0 under 8G and 4G; per-phase `MADV_RANDOM` cut the bytes to 0.16× (layout) and 0.07–0.15× (Leiden); on the whole Leiden run the advice slowed the aggregation scan 1.44–1.58×, and with a working set that fits it read 8.7× more. The cache (#178, same conditions, one Leiden level, deterministic): the in-RAM hash at 8 GiB; at equal resident memory (7.1 GiB) it read 305–331 GB against 263 GB for whole-run `MADV_RANDOM` on the map, so it does not replace the advice on local files yet. | accepted, partly validated: iterative scans yes; community detection in the lab, one case; production #60. Regime 2 for random access: the cache is built (#178); its time target is open (#240) |
| [D12](adr/0012-edge-map-framework.md) | **One edge-map framework** so each kernel is written once for all regimes and for WASM (many short calls). | PageRank loop body identical for slices and streams (copied in the prototype, not shared); V8 keeps a one-shot kernel in the baseline tier (4.6× slower: lj, 1 thread, Chromium, shared machine). In production (#44, PR #202) one kernel body runs over raw, compressed, mapped and streamed sources (tests); WCC on orkut, raw source, 16 threads, takes 1.13× the lab's time (official; the compressed rows failed on setup; `bench/results/edge-map/20261126T043042Z-c0d7f248bb47.jsonl`). | accepted, partly validated (#240) |
| [D13](adr/0013-threads-per-kernel.md) | **Thread count is chosen per kernel.** Physical cores for memory-bound scans; all hardware threads for compute-bound kernels. | From 16 to 32 threads (idle): no gain for PageRank, components, BFS, k-core on orkut; triangles 2.51 → 1.46 s; async Louvain on orkut 0.88 → 0.65 s. Not general: async Leiden on lj 0.76 → 0.78 s, and every deterministic community run is slower at 32 (lj and orkut). Friendster PageRank looked flat beyond 8 threads, but only on a loaded machine. | accepted, partly validated (#22) |
| [D14](adr/0014-deterministic-by-default.md) | **Deterministic by default.** Same input + seed → same output for any thread count, any platform, any memory budget and whichever way the planner executes it (in memory or out of core; a view as a mask or materialized). The planner never substitutes a lower-precision variant by itself. "Same input" is the same stored graph, its order included: not a graph whose ids `compact --order` renumbered. Nodes without edges change no community. A map built again over an earlier map depends on it (the super level of ADR 0016 keeps the earlier choice inside its band). Not covered: results cut short by `timeout`, and the opt-in `fast` mode for community detection. The price in quality: none measured against `fast` mode on lj and orkut (mean modularity over five seeds within 0.003 either way, below the spread from seed to seed; exploratory, ADR 0014). The table of what is guaranteed, shown in the lab or not promised is in ADR 0014, "Reproducibility". Widened on 2026-11-25 (`design/DECISIONS.md`, "What 'repeatable' promises"); narrowed only by ADR. The supported platforms are x86-64 (any `target-cpu`), aarch64, i686 with SSE2 and wasm32 without `relaxed-simd`; i586/x87 and 32-bit ARM NEON floating point are not among them (Rust RFC 3514; `docs/research/README.md` N6). | Leiden/Louvain hashes identical at 1–32 threads (lj, orkut); layout and tiles byte-identical; PageRank and components agree across native and wasm; the pinned math, the exact accumulator and the counter-based RNG give the bits of their golden files (#187, #188; `crates/everygraph-core/tests/numerics_golden.rs`, wasm32 in CI); Leiden on a view as a mask and materialized, and with nodes without edges added, gives the same bits, and `compact` keeps it (`crates/everygraph-determinism/tests/leiden_views.rs`); identity across budgets and regimes not yet measured for kernels (tests: #54, #45, #47, #48, #93; the case that could narrow it: out-of-core Leiden, #60); cost at 16 threads (idle machine): Leiden 2.4–3.0×, Louvain 1.4–2.0×; #18's sub-round rule, adopted in #57, takes 0.59–0.73× the time of the current rule at 16 threads (lab, shared machine, exploratory; `lab/m1-04-det-leiden-scaling/README.md`). | accepted, partly validated (#54, #60); cost above target |
| [D15](adr/0015-leiden-visiting-order.md) | **Leiden with block-permuted visiting order and an explicit connectivity guarantee**; 1 iteration = fast, 3 = quality. | Order matters more than Louvain vs Leiden: id order costs 2.0–2.1 modularity points at one iteration (idle, 16 threads; 2.6–4.2 in earlier shared runs), the two algorithms differ by ≤ 0.4; block order takes 1.38–1.63× the time; 3 iterations land within 0.74% of the best reference (lj, orkut). | accepted, partly validated: 2 social graphs (#24, #259) |
| [D16](adr/0016-two-community-outputs.md) | **Two community outputs, one artifact:** the partition T (best modularity) and a map tree built from it by recursive sub-clustering (local modularity per region; a super level above T by a 90%-cover rule; one "+N smaller communities" aggregate per parent). T is a flagged antichain of the tree, never a depth. | Natural hierarchies have parents with up to 102,821 (lj) and 338,769 (orkut) children. Recursive sub-clustering: fan-out at most 64, 99.7% of parents in 4–64 on Friendster and 90.7–99.9% on four smaller graphs; T kept exactly (the cap baseline loses 1.6–4.3 modularity points); Friendster shaping under `MemoryMax=16G` in 499–506 s, anonymous peak 10.0 GiB (official, `bench/results/hierarchy-shaping/`; FINDINGS § M1-05). | accepted, amended 2026-11-26 (FINDINGS § M1-05); levels below T less stable than T (#181), leaves that cannot be split (#182) |
| [D17](adr/0017-layout-disc-packing.md) | **Layout = hierarchy-constrained disc packing**, deterministic; siblings placed by stress on the parent's quotient graph (#67, after #21). It shows local and community structure; distance between large regions carries little meaning, and the map's text and legend do not suggest otherwise. | 3.3 s for 4 M nodes (lj, 32 threads, shared machine); median edge length ÷ random-pair distance 0.014 vs 0.244 for an embedding layout (a blob); node discs cover about 1–3% of the map (estimate, not measured). Against DRGraph and ForceAtlas2 on lj and orkut (#175, deterministic metrics, exploratory times): ahead on local and community structure (NP₁ 16–50×; ground-truth 10-NN agreement 0.79 vs 0.55 lj, 0.52 vs 0.04 orkut), behind on global structure (Spearman layout vs hop distance 0.03 vs 0.31 lj, 0.55 orkut; edges between top regions, 21–24% of edges, as long as random pairs). sfdp and FM³ did not complete under 24 GiB on lj; BatchLayout was killed at 16 GiB with its default start. Stress per parent: about 1.8× solve CPU, byte-identical at 3 and 8 threads (lj). | accepted, partly validated (#67) |
| [D18](adr/0018-data-tiles.md) | **Data tiles, not pixels,** in PMTiles v3; dependency-free WebGL2 viewer; static mode and live mode. | lj, sizes: first view 152 KB / 18 requests; tiles median 4–6 KB; search in 2 requests. Not measured: live mode, integrated GPUs, Firefox, Safari. Above 117 M edges, only the Friendster tracer (#15, idle, RTX 4090): first view 163,184 B / 18 requests, 6.84 GB file, median tile 6.8 KB. | accepted, partly validated (#77, #69) |
| [D19](adr/0019-python-pyo3-arrow.md) | **Python via PyO3 abi3 + Arrow PyCapsule, bulk columnar API.** | Zero-copy verified; one abi3 wheel for CPython 3.10–3.14 (size 525 KB: estimate, not in a result file); per-call cost 28–40 ns (lj, shared machine). The first slice of the product package (PR #229, part of #81): an abi3-py310 wheel built by maturin; numeric result columns lent to Arrow without a copy; the GIL released around the build and every kernel (tests, no timing). | accepted |
| [D20](adr/0020-networkx-backend.md) | **NetworkX backend is an adoption channel, not the product**, built after v0.1 (#86; `design/preview/choices.md` D5). | lj, 3 threads, shared machine: 0.92–2.0× on an existing `nx.Graph` including conversion (conversion dominates); with native loading, kernel time only, PageRank 15.6–70× and components 24–53×. | accepted; #176 tests whether NetworkX's dispatched readers remove the conversion, which would re-decide the timing by ADR |
| [D21](adr/0021-queries-traversal-api.md) | **Queries v0.1: traversal API with budgets; no query language.** GQL/Cypher subset later, by bridging to one embedded engine (LadybugDB or Grafeo), not by building our own (ADR 0021, amended 2026-11-26). | Bidirectional BFS 0.3 ms on lj in-process, on raw arrays, single thread, shared machine (idle, #118: 0.13 ms raw, 0.16 ms on the compressed default, p50), vs 6.4 ms (Neo4j) and 0.3–4.8 s (embedded engines) through their query interfaces, 3 threads, shared machine — different measurement levels. | accepted, partly validated (#91) |
| [D22](adr/0022-browser-tier.md) | **Browser tier after v0.1:** wasm32 threaded build, ≤ ~100 M edges. | lj end to end in ~3 s in Chromium (threaded build, shared machine); ceiling at 265 M edges; Chromium and Node only. | accepted, partly validated (#108) |
| [D23](adr/0023-licence-and-open-specs.md) | **Licence Apache-2.0 OR MIT; published format specs.** | Kùzu, Memgraph MAGE, CozoDB lessons. | accepted |
| [D24](adr/0024-visual-system.md) | **One visual system ("Vernier") for viewer, docs, notebook, terminal and figures**: warm paper and soft black, one ink-blue accent, every number in monospace, the two-ring mark; values in `design/system/tokens.css`, rules in `design/preview/style/SPEC.md`. | Three rounds of previews; data palette and text contrast measured (worst categorical pair ΔE 15 normal vision, ~7.3 colour-blind; all text tokens ≥ 4.5:1). | accepted (approved by the owner, 2026-11-25) |
| [D25](adr/0025-user-facing-choices.md) | **User-facing choices are fixed in `design/DECISIONS.md`**: API shape and names, CLI messages, graph-semantics defaults, map style (bubbles, dark canvas, links for the selection only, five reused region colours, artefact regions drawn as such). | Owner approved the previews in `design/preview/` as a whole. | accepted; each can be reopened when built |

## 4b. How it looks and reads

The design is not described here but shown: `design/preview/README.md` indexes the
previews (viewer, API, command line, choices), `design/preview/style/` holds the full
style guide, and `design/DECISIONS.md` records what was chosen. Issues that build
something a user sees or types carry a "Design reference" paragraph pointing to the
exact page.

## 5. What is not solved yet

Each item names the issue that owns it.

1. **Community detection beyond RAM works in the lab for one case, not yet in the
   product, and the headline has not been run end to end.**
   - *The lab case* (#16, FINDINGS § M1-02; idle machine;
     `bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl` and
     `…20261125T174959Z-b61036e59740.jsonl`): one iteration of Leiden, async and
     deterministic, on Friendster, 16 threads, f64 weights, level 0 in the D4 compressed
     store, against the same algorithm in RAM (74.8 s). Aggregation is one sequential pass
     into budgeted buckets, and every quotient graph is spilled as a compressed store.
     - Under 16 GiB: 87.4 s, 1.17×, 0 disconnected communities, async Q median 0.6014 (one
       of two repetitions 0.5981, inside the run-to-run spread of 0.598–0.607). The
       deterministic mode gives the in-RAM partition (same hash, Q 0.6055).
     - Under 8 GiB: 175.3 s, 2.34×, the smallest budget within 2.5×. Under 4 GiB it still
       completes, at 3.49×. Below 16 GiB the cost is level-0 local moving, which re-reads
       the store 6–13× per sweep in ADR 0015's block-permuted order.
     - Map-shaped Leiden completes under 16 GiB: 164.1 s, Q 0.5900, 27 top communities.
     - Not measured: more than one iteration; the deterministic mode below 16 GiB; other
       graph families at this scale (orkut scaled down: 3.10×). A seeded relabelling at
       ingest kept Friendster at 1.28–1.43× under 8 and 4 GiB (#146); ADR 0015 is
       unchanged.
     - Owner of the product: #60.
   - *The headline "1.8 B edges → map on a 16 GB laptop"* (tracer, #15, FINDINGS § M1-01;
     Friendster, lab binaries, 32 threads, idle, uncompressed CSR;
     `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`): the lab chain does not
     complete under 16 GiB. With 56 GiB it completes in 455.5 s of stage time (sum of
     medians), with the capped Louvain hierarchy. Three blockers, each with its owner:
     - *Communities with a map-shaped hierarchy.* In the tracer, Leiden and fan-out-capped
       Louvain were killed in the first aggregation while thrashing on the mapped graph
       (1.8–2.7 TB read), and every level's quotient graph was kept in memory. Plain
       Louvain fits (4.2 GiB, Q 0.6038) but has a parent with 18.7 M children, which the
       layout cannot use. Answered in the lab by the case above and by the shaping of #19
       (FINDINGS § M1-05, official): recursive sub-clustering of the Friendster partition
       under `MemoryMax=16G` in 499–506 s, anonymous peak 10.0 GiB. Owners of the product:
       #60 (out-of-core Leiden), #62 (levels streamed to disk), #63 (shaping bounded in
       memory).
     - *The layout loads the graph into the heap.* Killed under 16 GiB that way. With the
       graph memory-mapped it fits (3.7 GiB anonymous) at 5.8× the time, re-reading
       733 GB. Owner: #67.
     - *The tile builder keeps every tile in RAM,* and each zoom level's working state:
       29.1 GiB anonymous with 56 GiB. Under 16 GiB (13.9 GiB anonymous, one pass over the
       graph per zoom level) it was cut by the 60-minute timeout before the last zoom
       level. Owner: #69.
2. **Map hierarchy quality.** Recursive sub-clustering of the partition T bounds fan-out
   at 64, with 90.7–99.9% of parents in 4–64 on five graphs, and keeps T exactly (#19,
   FINDINGS § M1-05; D16). Open: levels below T are less stable than T (#181); some leaves
   cannot be split (#182). The product is #63.
3. **Edges on the map.** Fog inside giant communities; 90% of edge weight undrawn on orkut
   at the deepest zoom (FINDINGS § E). Owners: #20, then #73.
4. **Cost of determinism for Leiden:** 2.4–3.0× at 16 threads on orkut and lj (idle
   machine); it stops scaling at about 8 threads. The sub-round rule of #18 takes
   0.59–0.73× the time of the lab's first rule at 16 threads (exploratory,
   `lab/m1-04-det-leiden-scaling/README.md`). Its price in modularity against `fast` mode
   is below the spread from seed to seed on lj and orkut (ADR 0014). Owner: #57.
5. **Memory-bandwidth ceiling** on graphs whose state exceeds the cache. Owner: #22.
6. **The default storage outside RAM, and in the build.** In RAM it is measured: kernels,
   Leiden and single-thread point queries on the default compressed storage, against raw
   arrays in the same binary, on four social graphs (#118, FINDINGS § M1-13); the default
   holds with the conditions written into D4. Not measured on the default storage: a
   streaming reader out of core (#46); point-query throughput at 16 threads (#90); web
   graphs, weighted and directed kernels (#24). The lab's ingest timings are from plain
   text to raw arrays without `fsync` (with it, lj took 0.68 s instead of 0.37 s in one
   run). The product's build writes the default storage with a streaming encoder under a
   budget (#35, tests); in exploratory runs it misses its speed targets, which are #240's.
7. **Evidence is narrow:** two social graphs for community quality; no weighted, directed
   or bipartite community runs (#24); no string-id input at Friendster scale and no
   Parquet ingest (#25, #33); no Firefox or Safari (#26).
8. **Non-technical.** Two granted patents have elements near the tile pyramid
   (`docs/legal/patent-check.md`), and a US trademark application for the name is pending
   (`docs/legal/name-check.md`); the owner accepted both risks, and professional opinions
   are advisable before the release (#122). One maintainer (#115). The short name `eg` is
   a package on PyPI, crates.io and npm, so the command is `everygraph`.

## 6. Success criteria for v0.1

| Proof | Target |
|---|---|
| 1 M-edge CSV → first map, laptop | < 10 s |
| Friendster ingest, 8 GB budget | < 60 s |
| Friendster PageRank under a 2 GB limit | within 1.35× of in-RAM |
| Friendster communities under a 16 GiB limit | no OOM; quality mode (3 iterations) within 1% of the modularity of the better of NetworKit PLM and GVE-Leiden, both run on this machine on the same graph with the same preprocessing; the fast mode's gap to that reference stated |
| Same graph at 4 and 32 threads | identical partition, layout and map file |
| Leiden quality (3 iterations) | within 1% of the best reference on 6 graph families; the reference is named per graph and run on this machine (lj: igraph Leiden, 0.7695 in one run, FINDINGS "Idle-machine re-runs") |
| Map of a 1 B-edge graph from a static URL | first view < 250 KB; 60 fps on a discrete GPU, usable on integrated |
| Any budgeted run | never OOM-killed |

**Why the community references are same-machine runs** (`docs/research/README.md` A2, A3;
`docs/research/sota-communities.md` § 3). A fixed modularity is not a quality claim. On
Friendster, published values are 0.622 for sequential Louvain and 0.627 for NetworKit PLM
(DSLM, Hamann et al. 2018, mean of 10 seeds, their 2 × 14-core hosts) and 0.626 for
Grappolo; ours at one iteration are 0.6038–0.6055 (FINDINGS § M1-01, § M1-02). Three
iterations have not been run on Friendster; #57 and #61 measure them with the references.
On lj the reference is igraph's 0.7695 (one run of two iterations on this machine); our
three-iteration 0.7647 is 0.6% below it. Traag et al.'s 0.7739 is the best of 10
replications × 10 iterations on their machine, so it is shown beside the gate, not used
as it.

**Where each target stands.** The targets are unchanged.

- *Friendster PageRank under a 2 GB limit, within 1.35× of in-RAM* (#118, idle, FINDINGS
  § M1-13). Through plain mmap of the compressed store, 16 threads: 4.31 s/iteration
  against 3.19 in RAM, 1.35×, no margin (arithmetic on a one-repetition, 5-iteration
  capped run that includes the initial degree pass; raw arrays the same way: 1.32×). The
  lab's raw-format streaming reader gave 1.08× (3.64 against 3.37 s/iteration, 32
  threads). The gap is a streaming reader for the compressed format: #46.
- *Friendster ingest, 8 GB budget, under 60 s* (benchmark harness, idle, 32 threads, no
  `fsync`, one untimed warm-up first;
  `bench/results/headline/20261125T114658Z-0ffe756ac9ac.jsonl`, rows 4–6; ADR 0026). The
  lab ingest to raw arrays took 36.97 s (35.06–37.14, median of 3) at the drive's
  sustained write rate. Encoding to the default format took 3.41 s at 16 threads plus
  4.77 s to write it with `fsync`, measured separately (#118): 40.4 s by addition before
  the write and 45.2 s with it. That encoder holds input and output in RAM, so it cannot
  run under 8 GB. The product's build encodes under the budget and writes the default
  format directly (#35); its time on Friendster is not measured yet (#240).
- *Friendster communities under a 16 GiB limit* (item 1 of § 5). In the lab, one iteration
  completes under 16 GiB with 0 disconnected communities at 1.17× the same algorithm in
  RAM; Q 0.6055 in the deterministic mode. The quality mode and the references are not
  measured. Owners: #57, #60, #61.
- *The headline "1.8 B edges → map on a 16 GB laptop"* (§ 5, item 1) also needs a layout
  that never loads the graph into the heap (#67) and a tile builder that bounds what it
  holds in RAM (#69).
- *Map of a 1 B-edge graph from a static URL, first view < 250 KB* (tracer, #15,
  `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`, row 31): 163,184 B in 18
  requests on the Friendster map (RTX 4090, headless Chromium). Frames 16.67 ms mean, none
  over 33 ms: capped by vsync, so 60 fps held on that GPU but the headroom is unknown.
  Integrated GPUs are not measured (#26).
