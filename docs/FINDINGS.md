# graphlab findings — evidence behind the Everygraph design

Feasibility lab run on 2026-11-24 on one workstation (Ryzen 9 7950X 16c/32t, 61 GB RAM,
Samsung 990 PRO NVMe, RTX 4090 unused by the pipelines). Datasets: com-LiveJournal
(`lj`, 4.0M nodes / 34.7M edges), com-Orkut (`orkut`, 3.1M / 117.2M), com-Friendster
(65.6M / 1.806B), twitter-2010 (41.7M / 1.468B arcs, directed).

**Read this first.** Eight experiments ran concurrently on the same machine. Unless a
number is marked *idle re-run*, it was taken under load (load average 20–40) and is
pessimistic and noisy (±30% typical, more for 32-thread and cold-I/O runs). Relative
orderings were stable. Extrapolations are marked as such. Code and raw results are in
`~/graphlab/exp/<experiment>/`.

---

## R — landscape and prior art (web research)

1. **Nobody covers the full stack.** No product combines ingest, beyond-RAM storage,
   parallel analytics, query and visualization above 10^7 edges in one embeddable,
   permissively licensed package. The closest map prior art: Uncharted Salt/Aperture-Tiles
   (Spark/HBase, 2.3M nodes / 9.9M edges, archived 2023, "patent pending" in 2015 — needs a
   patent check) and MSAGLJS tile pyramids (arXiv 2605.17498, May 2026: 32,768 nodes /
   236,978 edges).
2. **The incumbent bar rose in 2026.** Gephi 0.11 claims up to 10M elements with a new
   OpenGL engine (vendor claim). DuckDB 2.0 recursive CTEs with `USING KEY` claim ~40× on
   reachability. Grafeo (Rust, Apache-2.0, embeddable, WASM, GQL) and LadybugDB (Kùzu fork,
   v0.22.3) ship fast. We must win above 10^8 edges, on the map, on analytics breadth and on
   reproducibility — not on drawing 10^6 points.
3. **Determinism is feasible and a differentiator.** Recipes: Mt-KaHyPar synchronized move
   application (arXiv 2112.12704); LD-Leiden snapshot + per-thread buffers + serial commit
   (arXiv 2502.18497).
4. **Beyond-RAM is backed by literature.** ACGraph (SIGMOD'25): semi-external, 4 KB blocks,
   async io_uring. GBBS: compression lets 225B edges fit in 330 GB. Cologni & Vigna (WAW
   2026): new webgraph-rs codecs up to 15% smaller than BV, up to 54% faster random access
   than Zuckerli.
5. **PMTiles v3 can carry arbitrary payloads** (tile type 0x00, 64-bit offsets, Hilbert
   ids, JSON metadata, 16 KB bootstrap request) — confirmed in practice by experiment E.
6. **Distribution channels are unclaimed.** NetworkX backends: only GPU-only nx-cugraph
   thrives (134 of 797 dispatchable functions); graphblas-algorithms unreleased since
   2023-10. Polars has only toy graph plugins. graph-tool still has no PyPI wheels.
7. **Governance is a feature.** Kùzu was acqui-hired (archived 2025-10-10); CozoDB, IndraDB,
   lance-graph dormant; Memgraph merged MAGE into its BSL core. Permissive licence, a
   published format spec and a foundation path matter.
8. **Query: reuse.** Rust parsers exist (`ocg`, Grafeo's GQL parser); DataFusion for
   relational operators. SQL/PGQ was reverted from PostgreSQL 19 (2026-09-07). Full GQL is
   not a v1 priority.
9. **Funding** (as of 2026-11): NLnet NGI Zero Commons final call closed 2026-06; Sovereign
   Tech Fund rolling (infrastructure, €50k–1.3M); NumFOCUS small grants ≤$10k; ERC PoC
   ~€150k.
10. **Unverified items** to re-check before citing publicly: AWS–DuckDB Labs deal, exact
    webgraph-rs licence, whether the Uncharted patent was granted, SNAP-tFDP code
    availability.

## G — browser / WebAssembly tier: GO with conditions

- The same Rust core runs in the browser: wasm32 + `wasm-bindgen-rayon`, CSR built in two
  passes from a dropped `File`, all in a worker, COOP/COEP headers. Results are identical
  across native / wasm32 / wasm64 / Node / Chromium and 1–32 threads.
- lj end to end in Chromium (drop file → CSR → PageRank → CC): ~3 s vs ~1.3 s native.
  PageRank 20 it.: native 0.33 s (32t); Chromium threaded 0.52 s (16t) — 0.44 s with the
  optimizing tier forced.
- **V8 tiering trap:** a kernel executed as one long call stays in the baseline compiler
  (up to 9× slower: 25.3 s vs 5.5 s single-threaded). Kernels must be many short calls;
  ship only the threaded build.
- **Ceilings:** 4 GiB linear memory *and* no single allocation above 2 GiB. Two-pass build
  fits ~265M undirected edges; honest promise 10–50M "in seconds", ~100M defensible.
  memory64 is ~3× slower on PageRank and absent from Safari: do not rely on it.
- OPFS works as a spill/persistence layer (684 MB/s write) but there is no mmap.
- **Constraints on the core from day one:** no `std::fs`/mmap/threads inside algorithms
  (I/O behind a chunked, positional, re-readable byte-source trait); two-pass builders;
  rayon behind a facade; u32 node ids with u64 offsets on disk; segmentable arrays
  (≤ 2 GiB chunks); fallible, pre-estimated allocations; restartable API (panics abort).
- Toolchain: threads need a pinned nightly + `-Z build-std` and explicit linker flags
  (`--shared-memory --import-memory …`); module is 189 KB.

## F2 — query-engine bar, Python bindings, NetworkX backend

Query bar on lj, 3 threads, warm (answers cross-checked against igraph):

| engine | load s | neighbors ms | 2-hop ms | shortest path ms | PageRank |
|---|---|---|---|---|---|
| python-igraph 1.0 | 7.9 | 0.0006 | 0.48 | 291 | 8.3 s (1t) |
| rustworkx 0.18 | 3.6 | 0.0018 | 0.035 | 4,785 | n/a |
| LadybugDB 0.22.3 | 12.1 | 0.09 | 0.64 | 2,550 | algo ext. failed to load |
| DuckDB 1.3.2 + DuckPGQ | 1.2 | 0.45 | 4.7 (SQL) | OOM > 6 GB | 7.0 s |
| GFQL (polars) | 0.23 | 268 | 1,206 | no operator | – |
| Neo4j 5.26 CE + GDS | 51 | 2.6 | 11.4 | 6.4 | 10.0 s (+5 s projection) |
| lab Rust kernel | – | – | – | – | 1.07–1.33 s |

- DuckPGQ does not work on DuckDB 2.0.1 (parser error); GFQL scans the whole edge frame
  per hop. Embedded engines are weakest at shortest path.
- **Python binding (PyO3, abi3-py310, maturin):** zero-copy verified by buffer address for
  NumPy views into the mmap and for Arrow PyCapsule export into pyarrow / Polars / pandas
  (4M-row result → table in 0.5–4.4 ms). GIL released; Python = native speed. One 525 KB
  wheel for CPython 3.10–3.14 (free-threaded builds need separate wheels). Per-call cost
  28–62 ns: the API must be bulk/columnar.
- **NetworkX backend (~200 lines):** dispatch, fallback and zero-code-change routing work.
  On an existing `nx.Graph`, conversion dominates: lj PageRank 2.0× including conversion
  (23.4 s to convert), 29–38× on the cached graph; CC 0.92× including conversion,
  121–177× kernel-only. Semantic traps: PageRank `tol` scaled by N, default
  `weight="weight"`, arbitrary hashable labels, insertion order. Coverage needed for a
  credible claim: ~100–150 of 797 functions (2–3 months for ~60, 6–9 for ~150).

## E — the map: layout, tiles, viewer

| | lj | orkut |
|---|---|---|
| layout 32t / 1t | 3.3 s / 23.1 s | 8.1 s / 51.3 s |
| layout peak RSS | 0.88 GB | 2.1 GB |
| tile build 32t | 8.5 s | 15.0 s |
| tiles / single file | 173,017 / 851 MB | 63,846 / 460 MB |
| tile gzip median / max | 4.2 / 20.5 KB | 5.9 / 21.6 KB |
| first view | 152 KB, 18 requests | 143 KB, 18 requests |
| zoom-to-node session | 5.0 MB, 546 requests | 3.9 MB, 454 requests |
| search, cold | 2 requests, 47 KiB | 2 requests, 44 KiB |
| edge weight shown at deepest zoom | 53% | 10% |

- Hierarchy-constrained disc packing (area ∝ node count, small force solve per parent,
  derived intermediate levels when a parent has > 384 children) is cheap and readable;
  a FastRP-like embedding + PCA is a featureless blob (median edge length ÷ random-pair
  distance 0.244 vs 0.014).
- A real **PMTiles v3** file with custom payload works (official Python reader opens it).
- Layout and tile file are byte-identical across thread counts after one fix.
- Exact aggregates: 1,403,208 lj communities match the community experiment, 0 mismatches.
- Viewer: dependency-free WebGL2; 16.67 ms frames on the RTX 4090 (vsync-capped);
  SwiftShader 29–136 ms mean, 567 ms worst.
- **What does not work:** hierarchies with parents of up to 100,929 children force
  meaningless derived regions (identical "627 nodes" bubbles); discs cover only 1–3% of
  the map; inter-continent distances mean little (top quotient graph nearly complete);
  edges become fog inside giant communities; thread scaling of layout is only 6–7×.
  Hierarchy quality matters more than layout tuning (fallback hierarchy: 3× worse).
- Extrapolation to 1B edges (not measured): layout 2–4 min, ~20 GB RAM; 9–13 GB of tiles.
- Needed for readable edges: distance-adaptive aggregation, on-demand ego edges from a
  per-node adjacency index, hub suppression, bundling along the hierarchy.

## F1 — identifiers, point queries, properties, views

- **ID dictionary (65M–500M keys).** FST is the wrong default: on random keys (Ethereum
  addresses) it is 1.9× the raw key size and builds single-threaded (113–200 s for 65M).
  Minimal perfect hash (`ptr_hash`): 500M keys built in 10.8 s at 32t, 2.61 bits/key
  (163 MB), forward lookup 0.56 µs, batch mapping 132 M/s unverified. Front-coded sorted
  blocks give prefix search (3 µs) at 18 B/key for URL-like keys. Ingest bottleneck is
  the first dedupe pass: 15 M keys/s (≈4 min per 3.6B endpoints, extrapolated).
- **Point queries on mmapped CSR, warm, single thread (p50):** degree/neighbors
  0.1–0.6 µs; common neighbors 0.4–0.8 µs; 3-hop with 10k budget 56–115 µs;
  bidirectional BFS 0.3 ms (lj), 0.6 ms (orkut), 1.5 ms (twitter). p99 tails on hubs:
  up to 15 ms (2-hop) and 113 ms (path).
- **Cold / out-of-core:** neighbors 0.1–0.25 ms (2 I/Os; pin offsets in RAM to make it 1);
  frontier prefetch is essential (orkut 2-hop 8.7 → 0.47 ms); shortest path 25 ms p50 on
  10^8-edge graphs but 0.8 s on Friendster — needs resident topology or a landmark index.
  Under a 3 GiB cgroup on Friendster (15 GB): neighbors 105 µs p50.
- **Directed graphs:** CSC costs +4.23 B/arc (twitter +6.2 GB), built in 12.3 s at 32t.
- **Properties:** duplicate small hot columns per CSR entry; edge-id indirection makes
  kernels 1.7–2.1× slower. **Masks never beat the unfiltered scan**; a materialized
  filtered CSR (47–135 ms to build on orkut) gives PageRank at 0.09× for 1–10%
  selectivity. Planner rule: mask above ~20–30% selectivity, materialize below. Roaring
  only below ~5% density.
- **Temporal:** time-sorted lists make window scans 2–4× faster but lose sorted-merge
  intersection; prefer neighbor-sorted canonical CSR plus time-partitioned segments.

## B1 — Rust core: ingest, kernels, larger-than-RAM

Prototype: ~2,400 lines of Rust (`glab-core`, `glab-cli`), `lto=fat`, `target-cpu=native`,
no hand-written SIMD. The machine was never quiet (load 10–45): the same orkut PageRank took
1.01 s at load 36 and 0.46 s at load 13.

**Ingest (text edge list → validated symmetric CSR, 32 threads):**

| graph | budget | seconds | M edges/s | peak RSS | reference |
|---|---|---|---|---|---|
| lj | 8 GB | 0.37 | 93 | 0.74 GB | numpy 51 s; DuckDB 2.0.1 3.60 s (from .gz) |
| orkut | 8 GB | 0.90–1.22 | up to 130 | 2.2–2.4 GB | numpy 197 s; DuckDB 12.57 s (from .gz) |
| Friendster (32.4 GB text, 1.8B edges) | 32 GB | 20.1 | 90 | 13.2 GB | – |
| Friendster | 8 GB | 33.2 and 45.3 (two runs) | 40–54 | 3.6 GB | – |

- One code path for in-RAM and out-of-core (partition by source range into buckets, sort
  each bucket in parallel). Output is byte-identical to the reference CSR in every mode.
- gzip inflate is the bottleneck by 10–20× (0.3–0.6 GB/s vs 7–10 GB/s parse-only).
- Out of core the bottleneck is temp-file traffic (128 GB of NVMe I/O at an 8 GB budget),
  not CPU. O_DIRECT helps for input, **hurts** for small appending temp writes
  (74.6–81.4 s vs 33.2–45.3 s). Parallel `pwrite` into one output file does not scale.

**Kernels in RAM (orkut, 32 threads):** PageRank 20 it. 0.40–0.46 s (NetworKit 3.42 s);
WCC (Afforest) 27 ms (NetworKit parallel 302 ms); BFS 35 ms; k-core 0.27–0.42 s;
triangles 2.82 s. Friendster: PageRank 3.8–4.1 s/iter, WCC 0.55 s, BFS 0.43 s, k-core 9.3 s,
triangles 57.5 s.

- **Thread scaling is not proven near-linear.** Triangles 13.7× (1→32); PageRank/WCC/BFS
  3–6× on orkut; Friendster PageRank is flat from 8 threads up — consistent with a
  memory-bandwidth ceiling on a dual-channel desktop (not confirmed with counters).
  Cache-blocked PageRank or locality reordering is the only route past it.

**Larger than RAM (Friendster, 14.4 GB of targets, hard cgroup limits):**

| limit | PageRank f32 s/iter (semi-external / naive mmap) | RSS |
|---|---|---|
| none | 4.37 / 3.81–4.06 | 1.47 GB / 14.7 GB |
| 16 GB | 4.18 / 3.91 | 1.47 GB / 14.7 GB |
| 8 GB | 5.12 / 5.17 | 1.47 GB / 7.96 GB |
| 2 GB | 4.26 / – | 1.35 GB |

- Going out-of-core costs 0–35% for sequential kernels. Semi-external does not beat mmap
  on speed; it wins on a fixed, small, predictable RSS. WCC at 8 GB: 5.6–6.1 s.
- The only OOM was anonymous O(n) state (WCC at 1 GB). **Planner rule: choose the regime
  by the size of the O(n) state, not by graph size**, and account memory explicitly
  (mmap "RSS" is misleading).

**Probes (orkut PageRank):** mmap-warm = heap (mmap can be the only in-RAM mode);
u64 target ids cost +70–80% (keep u32 dense ids by default); node reordering gives nothing
on orkut (random order costs 15–25%), unmeasured on Friendster; huge pages ≤ 10%.

Prototype limits: original ids < 2^32, undirected, unweighted. Not measured: Friendster from
.gz, Parquet and string-id ingest, reordering on Friendster. Estimated effort to
productionize: 4–5 engineer-months.

## B2 — parallel hierarchical community detection (Louvain / Leiden)

Prototype: ~1,900 lines of Rust; GVE-style Louvain and Leiden with an async (racy) mode and
a deterministic mode over the same decision rule. All timings noisy (load 15–38); quality
numbers, hashes and level sizes are not affected. Modularity computed by one common routine.

| 32 threads unless noted | lj | orkut |
|---|---|---|
| Leiden async | 0.85 s, Q 0.7496, 0 disconnected | 1.56 s, Q 0.6743, 0 disconnected |
| Louvain async | 0.77 s, Q 0.7480, 87 disconnected | 0.75 s, Q 0.6623, 22 disconnected |
| Leiden deterministic (16t) | 2.57 s, Q 0.7480 | 4.06 s, Q 0.6760 |
| GVE-Leiden binary, same machine | 2.35 s, Q 0.7276 | 1.55 s, Q 0.6567 |
| best reference (NetworKit ParallelLeiden) | Q 0.7704 | Q 0.6911 |
| Leiden async, 3 iterations (dev cores) | Q 0.7652 | Q 0.6935 |

- **Visiting order matters more than Louvain vs Leiden.** Processing in id order (what GVE
  and PLM do) costs 3–4 modularity points and yields 3–4× more communities; a
  block-permuted order (blocks of 64 ids) fixes it at 1.5–2× the time.
- One iteration is 0.7–2.7% below the best reference; 3 iterations are within 0.7% (lj) or
  above it (orkut). Each extra iteration costs roughly one more run.
- Leiden had 0 disconnected communities in every run only thanks to an explicit
  split-last step (it fired: +27 communities on lj, +476 on Friendster).
- vs GVE (shared-machine runs suggested 2.3–3.4× faster; **the idle re-run below corrects
  this to 1.0–1.3× for Leiden and 2.2–2.6× for Louvain**), with 0.5–2.7 points more
  modularity. The published 400M+ edges/s was not reproduced by any implementation on lj.
- Aggregation is the scaling bottleneck: 35–40% of time at 16 threads, 58–69% on Friendster.

**Determinism (bit-identical output for any thread count):** hashed sub-rounds with frozen
decisions, commutative commit, smaller-label anti-oscillation rule, fixed-point convergence
sums, deterministic refinement and aggregation. Verified: identical FNV hash at
T = 1, 2, 4, 8, 16, 32 for Louvain and Leiden on both graphs.

| det / async time | T=4 | T=8 | T=16 | T=32 |
|---|---|---|---|---|
| Louvain | ~1.0× | 1.1–1.3× | 1.3–1.5× | 2.0× |
| Leiden | 1.4–2.0× | 1.9–2.3× | 2.3–3.0× | 2.7–3.6× |

Modularity cost ≤ 0.4%. "Deterministic with ≤ 30% overhead" holds for Louvain up to ~8
cores, **not** for Leiden (≈2× at 4–8 threads). Deterministic Leiden stops scaling at ~8
threads (coarse-level sub-rounds too small — untested fix: adaptive sub-round size).
Bit-identity needs exact totals: fine for unweighted/integer weights; f32 weights need
fixed-point totals (designed, not implemented).

**Hierarchy for the map:** Louvain is shallow with huge fan-out (orkut 3.07M → 9,042 → 115);
Leiden's refined levels shrink 3.5–18× per level but remain heavy-tailed (parents with up
to ~100k children). A fan-out cap (64) during coarsening plus level compaction (ratio ≥ 3)
bounds fan-out on all but the top 1–2 levels at a cost of 2–5 modularity points
(lj 0.750 → 0.727, orkut 0.685 → 0.631). Structural limit: top levels are star-like
(giants with thousands of satellite communities). Not tested: recursive sub-clustering,
nested resolution sweeps. Exact per-level aggregates verified independently.

**Memory / out-of-core:** O(n) state is small (21–53 B/node); the dominant term for Leiden
is the level-1 quotient graph (22–44% of input entries). Friendster: Leiden in RAM
154.5 s, Q 0.6033, 31,960 communities, 23.6 GB anonymous peak (does **not** fit a 16 GiB
limit). Louvain under a hard 16 GiB limit completes in 109.6 s with 3.97 GB anonymous, but
the partition is poor (Q 0.4647, undiagnosed). Scaled-down semi-external test (orkut,
limit ≈ edge array): 5.6× slower; below it, 51× slower or OOM. **Semi-external community
detection is not yet a product feature:** aggregation needs a sequential sort/bucket
implementation and the level-1 graph must be budgeted or spilled.

Not done: twitter-sym, directed graphs, deterministic mode at 10^9 edges, quality on
non-social graph families, Miri/TSan on the shared-memory code.

## C — topology storage and compression

Heavily shared machine; 32T rows are official runs under the lock, 6T rows are noisy.

**Codec bake-off (bits/edge incl. index; scan in M edges/s; one pull-PageRank iteration at 32T):**

| graph | codec | bits/edge | scan 1T | scan 32T | PR iter | random list |
|---|---|---|---|---|---|---|
| orkut | raw u32 CSR | 32.8 | 4,715 | 12,734 | 0.026 s | 113 ns |
| orkut | delta + Stream VByte | 16.7 | 1,757 | 9,773 | 0.051 s | 369 ns |
| orkut | BVGraph (webgraph-rs, w7) | 15.4 | 94 | 1,079 | 0.18 s | 1,713 ns |
| twitter-sym | raw | 33.1 | 5,055 | 13,623 | 0.83 s | 93 ns |
| twitter-sym | Stream VByte | 16.1 | 1,756 | 21,688 | 0.85 s | 300 ns |
| twitter-sym | BVGraph w7 | 13.9 | 120 | 990 | 2.9 s | 1,416 ns |
| uk-2005 (web) | raw | 34.7 | 3,843 | 12,347 | 0.128 s | 94 ns |
| uk-2005 | Stream VByte | 11.4 | 1,400 | 21,744 | 0.084 s | 273 ns |
| uk-2005 | BVGraph w7 | 2.1 | 282 | 4,221 | 0.28 s | 768 ns |
| uk-2005 | varint + zstd blocks (6T) | 2.4 | 688 | – | – | 44 µs |

- **Social graphs barely compress** (13.5–16.7 bits/edge whatever the codec); Stream VByte
  decodes 7–20× faster than BV for ~1–2 bits more. **Web graphs need bit-level coding or
  inter-list copying** (2.1–2.4 bits/edge); byte codecs stay ≥ 9.5.
- At 32 threads Stream VByte scans **faster than raw** on twitter-sym (raw hits memory
  bandwidth at ~54 GB/s); PageRank on twitter is the same for raw and byte codecs
  (random gathers dominate). On orkut raw PageRank is 2× faster.
- **Cold from NVMe (6T):** raw ~1.0 G edges/s; Stream VByte 1.24–1.77 G edges/s; BV is
  decode-bound (0.2–0.5). **Under an 8 GB cap on twitter-sym** raw (10 GB) re-reads disk
  every pass (1.2 G e/s) while Stream VByte (4.8 GB) stays resident: 6.4 G e/s, 5.2× faster.
- **Node ordering (orkut, BV bits/arc):** original 15.25; random 20.0; degree 17.75;
  BFS 14.98; flat community id 14.84; **LLP 11.32 (−26%)**, which also makes raw PageRank
  32% faster. A flat community id gains almost nothing; a multi-level lexicographic key is
  needed. Leiden-hierarchy order not tested.
- Gap formats are id-width agnostic (identical bits/edge with ids shifted by 2^33);
  offsets must be u64 or Elias–Fano (0.13–0.16 bits/edge measured).
- webgraph-rs: Apache-2.0 OR MIT since 0.7.0; best on web graphs and for LLP; but
  ~100 M arcs/s per core, four breaking releases in 12 months, and non-optional native
  dependencies that make WASM unlikely. **Recommendation: own codec (~400 lines: Stream
  VByte gaps in node-aligned ~64K-edge blocks + Elias–Fano offsets, 4 KiB-aligned
  sections) as the default storage; webgraph-rs as an optional import/export/archive
  feature.** This overturns the "raw by default, compressed when it does not fit" plan.

Not done: per-list Elias–Fano, SIMD prefix sums, orderings on twitter, io_uring streaming,
cold scans at 32 threads.

---

## Idle-machine re-runs (2026-11-24, nothing else running)

These supersede the shared-machine timings above where they overlap.

**B1 — ingest and kernels**

| measurement | idle result |
|---|---|
| lj / orkut text → CSR, 32 threads | 0.42 s / 0.89 s (1 thread: 1.22 s / 4.20 s) |
| parse only, 32 threads | 0.047 s (lj), 0.123 s (orkut) |
| from gzip (in-process / pigz pipe) | lj 1.24 / 1.14 s; orkut 3.93 / 3.66 s |
| Friendster, 32 GB budget | 21.1 s, peak RSS 13.2 GB |
| Friendster, 8 GB budget | 52.4 s, peak RSS 3.6 GB (33–45 s in earlier shared runs: variance is high) |
| orkut PageRank f32, 20 it. | 3.57 s (1T) → 1.87 → 0.96 → 0.53 → 0.37 (16T) → 0.38 (32T) |
| orkut PageRank f64, 20 it. | 0.43 s (16T) |
| orkut WCC | 81 ms (1T) → 13 ms (16T) |
| orkut BFS | 99 ms (1T) → 22 ms (16T) |
| orkut k-core | 1.35 s (1T) → 0.28 s (8T) → 0.30 (16T) → 0.37 (32T) |
| orkut triangles | 9.24 s (4T) → 1.46 s (32T) |
| u64 vs u32 targets, PageRank f32 | 0.70 s vs 0.38 s (+85%) |
| node order, PageRank f32 | original 0.38, BFS 0.38, degree 0.41, random 0.47 s |
| Friendster PageRank f32 in RAM (mmap) | 3.37 s/iter |
| Friendster PageRank, semi-external, no limit / 8 GB / 2 GB | 3.71 / 3.69 / 3.64 s/iter, RSS 1.35–1.47 GB, 3.9 GB/s read |
| Friendster WCC in RAM / semi-external under 8 GB | 0.34 s / 3.55 s |
| Friendster BFS | 0.34 s |

Scaling on an idle machine is better than the shared runs suggested (PageRank 9.7× at 16
threads on orkut), and SMT adds nothing. Leaving RAM costs ~10% for PageRank.

**B2 — community detection (wall seconds, best of N)**

| graph | algo, mode | T=1 | T=4 | T=8 | T=16 | T=32 | modularity |
|---|---|---|---|---|---|---|---|
| lj | Leiden async | 6.09 | 1.76 | 1.02 | 0.76 | 0.78 | 0.7497 |
| lj | Leiden det | 9.63 | 3.44 | 2.56 | 2.30 | 2.51 | 0.7480 |
| lj | Louvain async | 3.61 | 0.99 | 0.57 | 0.42 | 0.38 | 0.7483 |
| lj | Louvain det | 4.88 | 1.51 | 1.01 | 0.85 | 0.99 | 0.7453 |
| orkut | Leiden async | 15.47 | 3.73 | 2.09 | 1.43 | 1.24 | 0.6689 |
| orkut | Leiden det | 16.41 | 5.75 | 4.09 | 3.48 | 3.77 | 0.6760 |
| orkut | Louvain async | 10.53 | 2.37 | 1.37 | 0.88 | 0.65 | 0.6726 |
| orkut | Louvain det | 8.86 | 2.49 | 1.51 | 1.20 | 1.13 | 0.6684 |

- Price of determinism at 8 / 16 threads: Leiden 2.0–2.5× / 2.4–3.0×; Louvain 1.1–1.8× /
  1.4–2.0×. The earlier claim "Louvain within 30% up to 8 cores" holds on orkut only.
- Quality mode (3 iterations, 16 threads): async lj 2.00 s Q 0.7647, orkut 3.58 s
  Q 0.6922; deterministic lj 5.06 s Q 0.7651, orkut 7.06 s Q 0.6905, same hash at 16 and
  32 threads.
- Natural (id) order, 16 threads: Leiden lj 0.55 s Q 0.7302; orkut 0.88 s Q 0.6475 —
  faster, 2–4.5 points worse.
- GVE timings printed during this script were taken while it still shared the machine;
  the clean GVE numbers are in the baseline table below.

**A — reference tools on the same idle machine** (pinned: NetworKit 11.3, python-igraph 1.0.0,
scipy 1.18.1, rustworkx 0.18.1, DuckDB 2.0.1, GVE from puzzlef, largegraph 0.1). Graphs were
built from the same CSR arrays; modularity computed by one routine. GVE times exclude loading.

| tool (threads) | task | lj | orkut |
|---|---|---|---|
| NetworKit (32) | build graph from arrays | 7.8 s | 29.0 s |
| NetworKit (32) | PageRank, 20 iterations | 1.40 s | 2.79 s |
| NetworKit (32) | parallel components | 0.17 s | 0.25 s |
| NetworKit (32) | k-core | 0.26 s | 1.23 s |
| NetworKit (32) | BFS from the hub (sequential in NetworKit) | 0.85 s | 1.38 s |
| NetworKit (32) | triangles | 4.8 s | 24.0 s |
| NetworKit (32) | PLM | 4.9 s, Q 0.7485 | 6.6 s, Q 0.6731 |
| NetworKit (32) | PLM + refine | 7.6 s, Q 0.7635 | 10.0 s, Q 0.6864 |
| NetworKit (32) | ParallelLeiden | 6.0 s, Q 0.7688 | 9.4 s, Q 0.6720 (0.6882 at 1 thread) |
| igraph (1) | PageRank / components | 11.1 s / 1.04 s | 32.0 s / 1.71 s |
| igraph (1) | multilevel (Louvain) | 146 s, Q 0.7493 | 460 s, Q 0.6746 |
| igraph (1) | Leiden, 2 iterations | 41.9 s, Q 0.7695 | 126 s, Q 0.6924 |
| scipy (1) | PageRank 20 it. / components | 3.3 s / 1.45 s | 7.8 s / 3.70 s |
| rustworkx (1) | build / components / k-core | 10.5 s / 3.7 s / 16.2 s | – |
| GVE-Louvain (32) | communities | 0.82 s, Q 0.7218 | 1.70 s, Q 0.6606 |
| GVE-Leiden (32) | communities | 1.02 s, Q 0.7223 | 1.45 s, Q 0.6636 |
| DuckDB (32) | CSV → relabeled, symmetric, sorted adjacency in Parquet | 1.91 s | 5.37 s |
| largegraph 0.1 | full pipeline (prepare, 5 passes, report) | 31.8 s, Q 0.666 | 70.5 s, Q 0.633 |

Peak RSS: NetworKit 2.2 / 6.7 GB before community detection (up to 10 GB during it);
igraph 7.1 / 23.6 GB; largegraph 2.4 / 4.1 GB.

**What the comparison says (lab prototype, idle machine)**

- Ingest: 0.42 s / 0.89 s against DuckDB's 1.91 s / 5.37 s (4.5–6×). DuckDB runs general SQL
  and writes compressed Parquet, so this bounds the gain rather than declaring a winner.
- Kernels on orkut against NetworKit at its best thread count: PageRank 0.37 vs 2.79 s (7.5×),
  components 13 vs 247 ms (19×), k-core 0.28 vs 1.23 s (4.4×), triangles 1.46 vs 24.0 s (16×).
- Communities against GVE: Leiden 0.76 vs 1.02 s on lj (+2.7 modularity points) and
  1.24–1.43 vs 1.45 s on orkut (+0.5); Louvain 0.38 vs 0.82 s and 0.65 vs 1.70 s.
- Communities against the quality leaders: three-iteration Leiden reaches Q 0.7647 in 2.0 s on
  lj (NetworKit ParallelLeiden 0.7688 in 6.0 s; igraph Leiden 0.7695 in 41.9 s) and Q 0.6922 in
  3.6 s on orkut (igraph 0.6924 in 126 s; NetworKit 0.6720–0.6882).
- The old prototype is 40–90× slower end to end than the new ingest + Leiden path and 8–11
  modularity points behind.
