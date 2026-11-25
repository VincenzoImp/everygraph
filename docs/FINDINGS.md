# graphlab findings — evidence behind the Everygraph design

Feasibility lab run on 2026-11-24 on one workstation (Ryzen 9 7950X 16c/32t, 61 GB RAM,
Samsung 990 PRO NVMe, RTX 4090 unused by the pipelines). Datasets: com-LiveJournal
(`lj`, 4.0M nodes / 34.7M edges), com-Orkut (`orkut`, 3.1M / 117.2M), com-Friendster
(65.6M / 1.806B), twitter-2010 (41.7M / 1.468B arcs, directed).

**Read this first.** Eight experiments ran concurrently on the same machine. Unless a
number is marked *idle re-run* or belongs to § M1-13, § M1-01 or § M1-02 (second-round experiments,
idle machine, 2026-11-25), it was taken under load (load average 20–40) and is
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
   236,978 edges). *Note 2026-11-25:* the literature reviews (`docs/research/README.md`,
   A1) found more prior art for the parts:
   - Uncharted's pipeline (Langevin et al. 2015): Louvain hierarchy → recursive in-parent
     layout → data-tile pyramid → web map;
   - a fan-out cap of 64 and recursive re-clustering (ASK-GraphView 2006);
   - compressed adjacency faster than raw in parallel (Ligra+ 2015);
   - Elias–Fano offsets (webgraph-rs);
   - seeded relabelling (DSLM 2018);
   - refusal before a run (Neo4j GDS).

   What remains ours is the combination. Within it, three properties were found nowhere
   else:
   - identity across threads, budgets, regimes and view representations;
   - a deterministic, budgeted text → CSR build;
   - a 1.8 B-edge map under a budget with declared omissions (an aim; not yet run end
     to end, § M1-01).
2. **The incumbent bar rose in 2026.** Gephi 0.11 claims up to 10M elements with a new
   OpenGL engine (vendor claim). DuckDB 2.0 recursive CTEs with `USING KEY` claim ~40× on
   reachability. Grafeo (Rust, Apache-2.0, embeddable, WASM, GQL) and LadybugDB (Kùzu fork,
   v0.22.3) ship fast. We must win above 10^8 edges, on the map, on analytics breadth and on
   reproducibility — not on drawing 10^6 points.
3. **Determinism is feasible and a differentiator.** Recipes: Mt-KaHyPar synchronized move
   application (arXiv 2112.12704); LD-Leiden snapshot + per-thread buffers + serial commit
   (arXiv 2502.18497). *Note 2026-11-25 (`docs/research/sota-kernels.md`,
   `sota-communities.md`):* LD-Leiden is repeatable only per thread count, not
   deterministic. Deterministic sub-round local moving is itself prior art (Mt-KaHyPar-SDet
   2022, DSLM 2018). What no work read offers is a Leiden, refinement included, that gives
   identical bytes for any thread count, platform and memory budget.
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
  (4.6× slower: 25.3 s vs 5.5 s with the optimizing tier forced; lj, PageRank, 1 thread,
  `lab/g-wasm/results/chromium.jsonl`. An earlier "up to 9×" compared 25.3 s with native
  2.81 s, which mixes tiering with wasm-vs-native). Kernels must be many short calls;
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
  (4M-row result → table in 0.5–4.4 ms). GIL released; Python = native speed. One abi3
  wheel for CPython 3.10–3.14 (about 525 KB — estimate, not in a result file; free-threaded
  builds need separate wheels). Per-call cost 28–40 ns for an empty method call
  (`lab/f2-python/results/goalB_lj_dev.json`, `goalB_lj_t32.json`): the API must be
  bulk/columnar.
- **NetworkX backend (~200 lines):** dispatch, fallback and zero-code-change routing work.
  On an existing `nx.Graph`, conversion dominates: lj PageRank 2.0× including conversion
  (23.4 s to convert), 29–38× on the cached graph; CC 0.92× (slower) including conversion,
  121× kernel-only (an earlier upper bound of 177× is not in the result file). With native
  loading (graph read by the engine, not converted), kernel time only: PageRank 15.6× at
  tol 1e-10 and 70× at the default tolerance, components 24–53×; with loading included,
  9.0× and 5.9× (`lab/f2-python/results/goalC_lj_dev.json`, 3 threads). Semantic traps: PageRank `tol` scaled by N, default
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
- Extrapolation to 1B edges, made before it was measured: layout 2–4 min, ~20 GB RAM;
  9–13 GB of tiles. *Corrected 2026-11-25 with the measurement (§ M1-01; Friendster, 1.8 B
  edges, 32 threads, idle, map-shaped Louvain hierarchy, uncompressed CSR in the heap, 56 GiB
  limit):* layout 69.6 s with 17.7 GiB anonymous; tiles 244.5 s with 29.1 GiB anonymous,
  a 6.84 GB file (`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`, rows 22–23
  and 29). Under 16 GiB the layout needs the graph memory-mapped (405.1 s) and the tiles do
  not finish.
- Needed for readable edges: distance-adaptive aggregation, on-demand ego edges from a
  per-node adjacency index, hub suppression, bundling along the hierarchy.

## F1 — identifiers, point queries, properties, views

- **ID dictionary (65M–500M keys).** FST is the wrong default: on random keys (Ethereum
  addresses as 20-byte binary) it is 1.9× the raw key size (1.03× as 42-byte hex text)
  and builds single-threaded (113–200 s for 65M). On URL-like keys it is the smallest form
  measured (0.28× raw). Minimal perfect hash (`ptr_hash`): 500M keys, MPHF built in
  10.8 s at 32t (whole dictionary with its key store 19.4 s), 2.61 bits/key (163 MB), forward lookup 0.56 µs, batch mapping 132 M/s unverified. Front-coded sorted
  blocks give prefix search (3 µs) at 18 B/key for URL-like keys. Ingest bottleneck is
  the first dedupe pass: 15 M keys/s (≈4 min per 3.6B endpoints, extrapolated).
- **Point queries on mmapped *uncompressed* CSR, warm, single thread (p50), shared machine:**
  degree/neighbors 0.1–0.6 µs; common neighbors 0.4–0.8 µs; 3-hop with 10k budget
  56–115 µs; bidirectional BFS 0.3 ms (lj), 0.6 ms (orkut), 1.5 ms (twitter). p99 tails
  on hubs: up to 15 ms (2-hop) and 113 ms (path).
  *Corrected 2026-11-25:* the same `f1-query` binary re-run unchanged on the idle machine
  (§ M1-13, `lab/m1-13-compressed/results/summary.txt` § 10, files `baseline_f1_*.json`;
  lj, orkut, twitter-2010 directed, p50) is 1.5–2.8× faster on bidirectional BFS:
  0.15 ms (lj), 0.40 ms (orkut), 0.53 ms (twitter); neighbors 0.14–0.22 µs, degree
  0.10–0.11 µs, common neighbors 0.27–0.54 µs, 3-hop with 10k budget 40–53 µs. The
  shared-machine figures above are kept for the record and are not to be quoted.
- **Cold / out-of-core:** neighbors 0.1–0.25 ms (2 I/Os; pin offsets in RAM to make it 1);
  frontier prefetch is essential (orkut 2-hop 8.7 → 0.47 ms); shortest path 25 ms p50 on
  10^8-edge graphs but 0.8 s on Friendster — needs resident topology or a landmark index.
  Under a 3 GiB cgroup on Friendster (15 GB): neighbors 105 µs p50.
- **Directed graphs:** CSC costs +4.23 B/arc (twitter +6.2 GB), built in 12.3 s at 32t.
- **Properties:** duplicate small hot columns per CSR entry; edge-id indirection makes
  weighted PageRank 1.7× (4 threads) to 2.1× (32 threads) slower — one kernel, orkut.
  **Bitset masks never clearly beat the unfiltered scan** (0.91–2.09× per PageRank
  iteration); a materialized filtered CSR (47–135 ms to build on orkut at 32 threads)
  gives PageRank at 0.09× for 1% selectivity and 0.09–0.17× for 10% (4 / 32 threads), and
  was faster per iteration than the mask at every selectivity measured (1–90%). The
  mask/materialize crossover depends on the number of passes and on memory; it was not
  measured (an earlier "mask above ~20–30%" was an estimate, not measured; #93 derives the
  rule). Roaring only below ~5% density.
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
- **Write-heavy ingest is bounded by the drive's sustained write rate, not by the CPU**
  (benchmark harness, idle machine, 2026-11-25; Friendster text → uncompressed CSR with
  `glab`, 32 threads, `O_DIRECT` input, output memory-mapped, no `fsync`, 48 GiB memory
  limit; Samsung 990 PRO 2 TB, ext4). Each run writes 45.1 GB at a 32 GB budget and
  55.4 GB at 8 GB (the cgroup's `io.write_bytes` in every row; `temp_disk` 29.6 / 39.9 GB
  plus the CSR). The first ingest after a light period took 17.7 and 19.5 s at 32 GB
  (92–102 M edges/s; repetition 1 of `bench/results/headline/20261125T101402Z-79634d76b95b.jsonl`
  and of `…/20261125T104322Z-79634d76b95b.jsonl`, rows 37) and used 14–16 cores on
  average; the back-to-back repetitions after it took 27.2–30.8 s and used 7.8–9.3 of 32
  cores (rows 38–39 of the same files). So the CPU was waiting, not saturated.
  - *Cause (inference with strong support, exploratory, not in a committed result file;
    recorded in the investigation of PR 136):* a 64 GB `O_DIRECT` write probe showed this
    drive writing its first ~10 GB after 5 idle minutes at 5.3 GB/s, then 1.4–1.6 GB/s, at
    49–55 °C with no thermal throttling. A 45–55 GB ingest therefore runs mostly in the
    slow region unless the drive has rested. `/proc/pressure/io` stall time tracks the
    slowdown: 9.7 s of stall in a 22.5 s ingest, 18.3 s in a 33.6 s one (exploratory runs).
  - *Ruled out (same exploratory investigation):* deleting the previous run's output
    before the next, write-back left over from the previous run, ext4 discard, drive
    temperature, and other load on the machine (the harness's disturbance check saw others
    at ≤ 0.31 cores and ≤ 0.5 MB/s in every official window).
  - *Protocol that follows:* a case that writes tens of gigabytes per repetition runs one
    untimed warm-up repetition (`warmup = 1` in `bench/specs/headline.toml`,
    `ingest-friendster`), so that every measured repetition meets the sustained rate that
    back-to-back ingests meet; the rested-drive figure is reported beside it, never instead
    of it. Under that protocol: 29.06 s at 32 GB and 36.97 s at 8 GB (§ Idle-machine
    re-runs). The lab's single Friendster runs (21.1 s and 52.4 s) had no such control.
  - Not measured: another drive, or the same ingest with `fsync`. What it implies: the
    ingest's speed on large graphs depends on bytes written, so the m2-08 reductions of
    temp traffic (no spill pass, 6-byte bucket entries) are speed work, not only space work.

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

- Going out-of-core costs 0–35% for PageRank, whose per-iteration cost in RAM happens to
  be close to the time needed to read the adjacency from this NVMe. It is **not** a general
  figure: a kernel that is cheap in RAM pays for the read (WCC: 0.34–0.55 s in RAM,
  2.8–6.1 s cold or streamed). Semi-external does not beat mmap
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
  and PLM do) costs 3–4 modularity points in these shared runs (6 dev threads: lj 4.2,
  orkut 2.6) and yields 3–4× more communities; a block-permuted order (blocks of 64 ids)
  fixes it at 1.5–2× the time. Idle, like for like (one iteration, 16 threads): 2.0–2.1
  points and 1.38–1.63× the time (see the idle re-run below).
- One iteration is 0.7–2.7% below the best reference in these shared runs (idle: 2.7% on
  lj, 3.2–3.4% on orkut); 3 iterations are within 0.7% (lj) or
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
to ~100k children in these runs; the idle re-run has 102,821 on lj and 338,769 on orkut,
`lab/b2-leiden/results/hier/`). A fan-out cap (64) during coarsening plus level compaction (ratio ≥ 3)
bounds fan-out on all but the top 1–2 levels at a cost of 2–5 modularity points
(lj 0.750 → 0.727, orkut 0.685 → 0.631; idle re-run 0.749 → 0.726 and 0.680 → 0.630). Structural limit: top levels are star-like
(giants with thousands of satellite communities). Not tested: recursive sub-clustering,
nested resolution sweeps. Exact per-level aggregates verified independently.

**Memory / out-of-core:** O(n) state is small (21–53 B/node); the dominant term for Leiden
is the level-1 quotient graph (22–44% of input entries). Friendster: Leiden in RAM
154.5 s (32 threads, shared machine), Q 0.6033, 31,960 communities, 22.6 GiB anonymous peak
(does **not** fit a 16 GiB limit). *Added 2026-11-25:* on the idle machine, one async
iteration at 16 threads takes 82.4 s on raw arrays and 82.0 s on the compressed default
(one repetition each, Q 0.6050 / 0.6008; § M1-13, `leiden_big.jsonl`). The two times are
not comparable with each other as a speed-up: different thread count and machine state. Louvain under a hard 16 GiB limit completes in 109.6 s with 3.8 GiB anonymous, but
the partition is poor (Q 0.4647, undiagnosed). *Not reproduced (2026-11-25, § M1-01, idle):*
plain Louvain under 16 GiB again (async, 1 iteration, f32 weights, `--lowmem`, 32 threads,
as here; this time on an idle machine and with level compaction `--min-ratio 3`) gave
Q 0.6038 with 33,306 communities, the largest 19.8 M nodes, against 35,682 and 33.4 M here
(`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 15;
`lab/m1-01-tracer/results/b2_louvain_plain_16G.json`;
`lab/b2-leiden/results/big/friendster_louvain_semiext16g.json`). One run each, cause not
diagnosed (#17). Scaled-down semi-external test (orkut,
limit ≈ edge array): 5.6× slower; below it, 51× slower or OOM. **Semi-external community
detection is not yet a product feature:** aggregation needs a sequential sort/bucket
implementation and the level-1 graph must be budgeted or spilled.
*Corrected 2026-11-25 by § M1-02 (#16, idle, Friendster, async, 1 iteration, 16 threads,
f64 weights, compressed level 0):* the 22.6 GiB need (written "23.6 GB" earlier) is that
of this lab's member-order aggregation, which holds the level-1 graph in memory. With
sequential aggregation into budgeted buckets and spilled quotient graphs, the same Leiden
runs under 16 GiB in 87.4 s, 1.17× the same algorithm in RAM (74.8 s), and under 8 GiB at
2.34×; 0 disconnected communities
(`bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl`,
`…20261125T174959Z-b61036e59740.jsonl`). The orkut figures above are superseded by the
same method: under 447 MiB (50% of the edge array) 3.10× the same algorithm in RAM, not
5.6× or 51×. What remains is the production implementation (#60), not feasibility.

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

- **Social graphs barely compress** (13.5–16.7 bits/edge whatever the codec, on orkut and
  twitter-sym; *corrected 2026-11-25:* with the same byte codec lj takes 18.1 and Friendster
  23.2 bits/edge, so "16–17 bits/edge on social graphs" does not hold in general — § M1-13,
  `lab/m1-13-compressed/results/encode.jsonl`); Stream VByte
  decodes 9.1–21.9× faster than BV for ~1–2 bits more (sequential scan, 1 and 32 threads;
  5.0–5.2× on uk-2005). **Web graphs need bit-level coding or
  inter-list copying** (2.1–2.4 bits/edge); byte codecs stay ≥ 9.5.
- At 32 threads Stream VByte scans **faster than raw** on twitter-sym (raw hits memory
  bandwidth at ~54 GB/s); PageRank on twitter is the same for raw and byte codecs
  (random gathers dominate). On orkut raw PageRank is 2× faster. *Corrected 2026-11-25:*
  not reproduced. With sequential access decoding batches of lists, orkut PageRank
  (20 iterations, f32, idle) takes 0.27 s compressed vs 0.37 s raw at 16 threads (0.73×)
  and 3.65 vs 3.66 s at 1 thread; decoding list by list costs 1.35× at 1 thread (§ M1-13).
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

The ingest rows are the benchmark harness's official idle runs of 2026-11-25
(`bench/results/headline/20261125T101402Z-79634d76b95b.jsonl` = run 1,
`…/20261125T104322Z-79634d76b95b.jsonl` = run 2, `…/20261125T114658Z-0ffe756ac9ac.jsonl`
= the Friendster re-take with one untimed warm-up repetition): `glab`, text → uncompressed
CSR, page cache warm for the text, no `fsync`, 8 GiB memory limit (48 GiB for Friendster),
each repetition a separate process. lj and orkut are the median of 6 repetitions (3 per
run). The lab's values of 2026-11-24 (`lab/b1-core/results/bench/ingest.jsonl`), now
superseded, were one run each, except lj and orkut at 32 threads (best of 3 runs in one
sequence; medians 0.428 / 0.889 s), and are kept in brackets. Its Friendster 32 GB run followed
only the small lj and orkut runs, so it met a rested drive (inferred from the order of
the file, not measured). Kernel rows are the lab's idle re-run of 2026-11-24.

| measurement | idle result |
|---|---|
| lj / orkut text → CSR, 32 threads, 8 GB budget, output memory-mapped | 0.386 s (0.370–0.395) / 0.781 s (0.754–0.806): 90 M / 150 M edges/s (rows 1–6 of runs 1 and 2; lab, best of 3: 0.42 s / 0.89 s) |
| lj / orkut, 1 thread | 1.182 s / 3.965 s (rows 31–36 of runs 1 and 2; lab: 1.22 s / 4.20 s) |
| parse only, 32 threads | 0.0467 s (lj), 0.118 s (orkut) (rows 10–12, 22–24; lab: 0.047 / 0.123 s) |
| from gzip (in-process / pigz pipe) | lj 1.134 / 1.054 s; orkut 3.70 / 3.549 s (rows 13–18, 25–30; lab: lj 1.24 / 1.14 s, orkut 3.93 / 3.66 s) |
| Friendster (1.806 B edges from 31 GiB of text, `O_DIRECT` input, output memory-mapped, 32 threads), 32 GB budget | **sustained** (after an untimed warm-up, back to back): 29.06 s (27.52–29.56, 3 runs), 62 M edges/s, peak RSS 13.3 GiB (re-take, rows 1–3). **Rested drive** (first ingest after a light period): 17.7 s and 19.5 s, 102 / 92 M edges/s (row 37 of runs 1 and 2). Lab single run, on a rested drive (inferred): 21.1 s, 86 M edges/s |
| Friendster, 8 GB budget | **sustained**: 36.97 s (35.06–37.14, 3 runs), 49 M edges/s, peak RSS 3.6 GiB (re-take, rows 4–6); runs 1 and 2 without a warm-up, right after the 32 GB case: 32.6–41.0 s (rows 40–42). Lab single run: 52.4 s (33–45 s in earlier shared runs); the gap to 37.0 s is not explained |

The spread between rested and sustained Friendster ingest is the drive's, not the code's:
see § B1, "Write-heavy ingest is bounded by the drive's sustained write rate".

| measurement | idle result |
|---|---|
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
threads on orkut). SMT adds nothing for PageRank, WCC, BFS and k-core, but helps triangle
counting (2.51 s at 16 threads → 1.46 s at 32) and asynchronous community detection on
orkut — not on lj (async Leiden 0.76 → 0.78 s), and deterministic community detection is
slower at 32 threads on both graphs. Leaving
RAM costs ~10% for PageRank and ~10× for WCC (I/O-bound). Ingest timings exclude `fsync`.

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
  block order costs 1.38–1.63× the time; id order is 2.0–2.1 points worse at one
  iteration (3.5–4.5 points against three iterations). One iteration is 2.7% (lj) and
  3.2–3.4% (orkut) below the best reference.
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
| NetworKit (32) | local clustering coefficient, non-turbo (not a plain triangle count) | 4.8 s | 24.0 s |
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

Peak RSS: NetworKit 2.1 / 6.5 GiB before community detection (up to 9.8 GiB during it);
igraph 6.9 / 23.1 GiB; largegraph 2.3 / 4.0 GiB (`ru_maxrss`, recorded in MiB).

**What the comparison says (lab prototype, idle machine)**

- Ingest: 0.386 s / 0.781 s (harness medians, 2026-11-25; the lab's single runs: 0.42 s /
  0.89 s) against DuckDB's 1.91 s / 5.37 s (4.9–6.9×). DuckDB runs general SQL
  and writes compressed Parquet, so this bounds the gain rather than declaring a winner.
- Kernels on orkut against NetworKit at its best thread count: PageRank in f64 0.43 vs 2.79 s
  (6.5×; 0.37 s in f32), components 13 vs 247 ms (19×), k-core 0.28 vs 1.23 s (4.4×).
  Triangle counting has no like-for-like reference here: the NetworKit row is its local
  clustering coefficient in non-turbo mode. All lab kernels ran on uncompressed arrays
  (for the same kernels on the compressed default, see § M1-13: orkut, 16 threads, PageRank
  f64 0.31 s, components 17 ms, k-core 0.22–0.36 s).
- Communities against GVE: Leiden 0.76 vs 1.02 s on lj (+2.7 modularity points) and
  1.24–1.43 vs 1.45 s on orkut (+0.5); Louvain 0.38 vs 0.82 s and 0.65 vs 1.70 s.
- Communities against the quality leaders: three-iteration Leiden reaches Q 0.7647 in 2.0 s on
  lj (NetworKit ParallelLeiden 0.7688 in 6.0 s; igraph Leiden 0.7695 in 41.9 s) and Q 0.6922 in
  3.6 s on orkut (igraph 0.6924 in 126 s; NetworKit 0.6720–0.6882).
- The old prototype is 40–90× slower end to end than the new ingest + Leiden path and 8–11
  modularity points behind.

---

## M1-13 — the default storage end to end (#118; 2026-11-25, idle machine)

**Question.** D4 makes compressed adjacency the default storage, but every kernel,
community and point-query number above was taken on uncompressed arrays, and the
compressed form had been measured only for scans and one PageRank iteration, on a shared
machine. What do the headline numbers become on the default format, and when should the
planner decode it to raw arrays?

**Setup.** Code and raw results: `lab/m1-13-compressed/` (`bench.sh`, `summarize.py`);
every table below is in `results/summary.txt` (section numbers given), every row in
`results/*.jsonl`, keyed by graph, algorithm, threads and configuration.

- *Format measured:* per node `[degree LEB128][Stream VByte of zigzag(first − v), then
  gaps − 1]` (byte-identical to lab C's Stream VByte records), node-aligned blocks of about
  64K entries, Elias–Fano over the n + 1 per-node byte offsets. All six stores (lj, orkut,
  twitter-sym, Friendster, twitter-2010 out and in) decode to the input arrays on every
  access path with all three decoders (`verify.jsonl`); kernel outputs (PageRank to 1e-9),
  query answers and deterministic-Leiden hashes are identical in every configuration.
- *Like for like:* one binary (commit `5c4f94f`, `target-cpu=native`), the same kernels,
  precision and thread count in three configurations: **A** raw arrays; **B** compressed,
  decoded on the fly; **C** compressed on disk with a raw copy decoded into RAM at open.
  Graph in anonymous memory unless a row says mmap; warm; kernel time only (open and decode
  reported separately); best of 3 unless noted. 1 and 16 threads.
- *Graphs:* lj, orkut, twitter-sym (2.41 B entries), Friendster (3.61 B entries);
  twitter-2010 directed with in-adjacency for point queries.
- *Idle evidence:* recorded run 03:40–06:33 (`results/logs/bench_run.log`). At the start
  of each of the 13 stages (`stages.jsonl`) the 1-minute load average was 0.99–2.99 and
  MemAvailable 58.8–59.5 GiB. Before every row, CPUs used by other processes over 0.5 s
  (`busy_cpus_before`, gate 0.5) were at most 0.48 over 557 rows, and
  `mem_available_gb_before` at least 57.3 GiB. The per-row load average reaches 24 from
  the experiment's own previous run, which is why the gate counts foreign CPU only. The
  `perf.jsonl` rows and the `baseline_f1_*.json` files were gated but do not carry the
  gate values. Two earlier attempts were discarded whole (experimenter's report on PR 133;
  their rows are not in the repository), one of them because a single-threaded job of
  another session skewed these kernels by 10–42% (`bench.sh` says 10–40%).
- *Baselines:* the lab prototypes re-run unchanged (summary § 10, `baseline_b1.jsonl`,
  `baseline_b2.jsonl`, `baseline_f1_*.json`) reproduce the idle re-runs above within 5%
  except orkut k-core at 16 threads (0.361 vs 0.30 s, +20%), orkut BFS at 16 threads
  (19.3 vs 22 ms, −12%) and orkut PageRank f32 at 1 thread (3.75 vs 3.57 s, +5.1%); B2
  within 7%; F1 is 1.5–2.8× faster than its shared-machine figures (corrected in § F1).
  Configuration A agrees with the lab binaries within 3% for kernels and Leiden; for point
  queries this harness is leaner (degree p50 0.02 vs 0.10 µs on lj, cause not isolated),
  so every A/B comparison below is inside this harness.

**Size and encode** (summary § 1, `encode.jsonl`; encode from an in-memory raw CSR, input
and output held in RAM; write includes `fsync`):

| graph | bits/edge (data + offsets) | raw | encode 16T | encode 1T | write + fsync |
|---|---|---|---|---|---|
| lj | 18.11 (17.69 + 0.41) | 35.7 | 0.064 s | 0.49 s | 0.11 s |
| orkut | 16.66 (16.54 + 0.12) | 32.8 | 0.168 s | 1.35 s | 0.31 s |
| twitter-sym | 16.11 (15.95 + 0.15) | 33.1 | 1.55 s | 14.7 s | 2.39 s |
| Friendster | 23.19 (23.02 + 0.17) | 33.2 | 3.41 s | 74.7 s (not trusted) | 4.77 s |
| twitter-2010 out / in | 16.54 / 15.94 | 33.8 | 0.93 / 0.99 s | – | 1.69 / 1.62 s |

"Bits/edge" is per stored entry (each undirected edge is stored twice). Friendster's ids
have no locality; it is the one social graph here above 18.1.

**Open** (summary § 5, `open.jsonl`; 16 threads; seconds, best of 3 warm, 2 cold):

| graph | A: read raw into RAM, warm / cold | B: open, warm heap / mmap / cold | C: open + decode, warm heap / mmap / cold | raw copy |
|---|---|---|---|---|
| lj | 0.024 / 0.058 | 0.012 / 0.010 / 0.029 | 0.038 / 0.034 / 0.055 | 0.29 GiB |
| orkut | 0.073 / 0.174 | 0.032 / 0.021 / 0.074 | 0.098 / 0.086 / 0.140 | 0.90 GiB |
| twitter-sym | 0.73 / 1.74 | 0.32 / 0.19 / 0.72 | 1.02 / 0.82 / 1.36 | 9.27 GiB |
| Friendster | 1.04 / 2.55 | 0.70 / 0.41 / 1.53 | 1.68 / 1.47 / 2.55 | 13.95 GiB |

The decode step alone at 16 threads, warm: 0.023–0.024 s (lj), 0.060–0.061 s (orkut),
0.60–0.68 s (twitter-sym), 0.92–0.99 s (Friendster), 2.9–4.0 G entries/s; at 1 thread
1.0–1.6 G entries/s. The raw copy costs 4 bytes per entry plus 8 per node of anonymous
memory. Cold, opening Friendster as C costs the same as reading the raw files (2.55 s both).

**Kernels** (summary § 2, `kernels.jsonl`, `kernels_big.jsonl`; seconds, A raw → B
compressed, ratio B/A; f32 and f64 are the PageRank state precision):

| kernel | T | lj | orkut | twitter-sym | Friendster |
|---|---|---|---|---|---|
| PageRank f32 ×20 | 1 | 1.40 → 1.95 (1.40) | 3.66 → 3.65 (1.00) | 94.9 → 105 (1.11; 1 rep) | not run |
| | 16 | 0.152 → 0.144 (0.95) | 0.366 → 0.266 (0.73) | 15.4 → 14.8 (0.96) | 67.2 → 63.9 (0.95; 2 reps) |
| PageRank f64 ×20 | 1 | 2.00 → 2.47 (1.24) | 4.32 → 4.11 (0.95) | 107 → 117 (1.09; 1 rep) | not run |
| | 16 | 0.211 → 0.193 (0.91) | 0.421 → 0.311 (0.74) | 19.3 → 18.8 (0.97) | 75.5 → 72.8 (0.96; 2 reps) |
| WCC (Afforest) | 1 | 0.059 → 0.056 (0.96) | 0.080 → 0.077 (0.96) | 1.99 → 2.48 (1.25) | 3.33 → 5.24 (1.57) |
| | 16 | 0.0107 → 0.0094 (0.88) | 0.0120 → 0.0172 (1.43) | 0.209 → 0.209 (1.00) | 0.365 → 0.497 (1.36) |
| BFS (direction-optimizing) | 1 | 0.073 → 0.136 (1.86) | 0.096 → 0.196 (2.04) | 1.11 → 1.97 (1.78) | 3.14 → 5.79 (1.85) |
| | 16 | 0.0152 → 0.0181 (1.20) | 0.0190 → 0.0280 (1.48) | 0.187 → 0.284 (1.52) | 0.356 → 0.508 (1.43) |
| k-core (PKC) | 1 | 0.569 → 0.580 (1.02) | 1.33 → 1.28 (0.96) | 16.1 → 16.7 (1.04) | not run |
| | 16 | 0.111 → 0.121 (1.09) | 0.22–0.36 in A, B and C (not trusted) | 5.26 → 5.50 (1.05) | 8.74 → 7.65 (0.88) |
| triangles, forward lists raw | 1 | 4.05 → 4.09 (1.01) | 33.1 → 33.5 (1.01) | not run | not run |
| | 16 | 0.294 → 0.290 (0.98) | 2.39 → 2.39 (1.00) | 69.8 → 69.4 (0.99; 1 rep) | 75.8 → 67.5 (0.89; 1 rep) |
| triangles, forward lists compressed | 16 | 0.324 → 0.328 (1.01) | 2.38 → 2.41 (1.01) | 78.2 → 78.4 (1.00; 1 rep) | 84.4 → 78.1 (0.92; 1 rep) |

- Ranges over the four graphs at 16 threads: PageRank 0.73–0.97 (both precisions), WCC
  0.88–1.43, BFS 1.20–1.52, k-core 0.88–1.09 (orkut excluded), triangles 0.89–1.01. At
  1 thread: PageRank 0.95–1.11 on orkut and twitter-sym, 1.24–1.40 on lj; WCC 0.96–1.57;
  BFS 1.78–2.04; triangles 1.00–1.03 (lj, orkut).
- C (raw copy decoded at open) is within 0.97–1.03× of A wherever it was run, except lj WCC
  at 1 thread (0.88×, faster), lj k-core at 16 threads (1.06×) and k-core on orkut at
  16 threads (noise, below).
- Triangle counting never reads the input at random: the lab kernel builds its own
  degree-oriented forward adjacency, so B costs two sequential decode passes. Keeping the
  forward lists compressed as well costs 1.00–1.12× with raw input and 1.01–1.16× with
  compressed input, and stores them in 17.6–25.4 bits per undirected edge instead of 32
  (`result.forward_bits_per_undirected_edge`). Counting without a derived adjacency is
  2.1× slower on orkut (5.14–5.33 s vs 2.39 s; `ablation.jsonl`, label `tc-naive`) and
  is not viable on twitter-sym (sum of degree² 2.5 × 10^14). At 32 threads orkut triangles
  take 1.44 s in A and in B (label `smt`).

**Leiden, 1 iteration** (summary § 3, `leiden.jsonl`, `leiden_big.jsonl`; seconds, graph
in heap; Q is the modularity of the best run):

| graph | mode | T | A raw | B compressed | B/A | Q (A / B) |
|---|---|---|---|---|---|---|
| lj | async | 1 / 16 | 5.97 / 0.764 | 6.54 / 0.792 | 1.09 / 1.04 | 0.7495 / 0.7495 (1T); 0.7496 / 0.7489 (16T) |
| lj | det | 1 / 16 | 9.47 / 2.31 | 10.1 / 2.29 | 1.06 / 0.99 | 0.7480 both |
| orkut | async | 1 / 16 | 14.3 / 1.41 | 15.4 / 1.47 | 1.07 / 1.04 | 0.6836 / 0.6836 (1T); 0.6813 / 0.6811 (16T) |
| orkut | det | 1 / 16 | 15.6 / 3.48 | 16.6 / 3.50 | 1.06 / 1.00 | 0.6760 both |
| twitter-sym | async | 16 | 29.1 (2 reps) | 29.2 (2 reps) | 1.01 | 0.4662 / 0.4626 |
| twitter-sym | det | 16 | 59.1 (1 rep) | 60.0 (1 rep) | 1.02 | 0.4790 both |
| Friendster | async | 16 | 82.4 (1 rep) | 82.0 (1 rep) | 0.995 | 0.6050 / 0.6008 |

Deterministic hashes are identical across raw, compressed, decoded, heap and mmap on every
graph run. C equals A (0.98–1.03×); the graph mmapped instead of in heap changes B by −4%
to +5% (lj, orkut). Only level 0 is compressed; the quotient graphs are the kernel's own raw arrays.
Leiden at 1 thread on twitter-sym and Friendster, and deterministic Leiden on Friendster,
were not run.

**Point queries** (summary § 4, `pq.jsonl`; warm, single thread, in-process, per-query
timer inside, graph in heap; 100,000 inputs for degree, neighbours and common neighbours,
10,000 for the rest; p50 in µs, A → B; in brackets the timer-free mean of the same queries
timed as one batch, which is the figure to use below ~0.2 µs, where p50 moves in 10 ns
steps):

| query | lj | orkut | twitter-sym | Friendster | twitter-2010 directed |
|---|---|---|---|---|---|
| degree | 0.02 → 0.04 [0.002 → 0.012] | 0.02 → 0.04 [0.001 → 0.013] | 0.09 → 0.18 [0.003 → 0.041] | 0.09 → 0.19 [0.003 → 0.049] | 0.09 → 0.20 [0.004 → 0.041] |
| neighbours | 0.03 → 0.06 [0.016 → 0.041] | 0.12 → 0.14 [0.045 → 0.116] | 0.11 → 0.19 [0.040 → 0.146] | 0.12 → 0.21 [0.046 → 0.159] | 0.11 → 0.17 [0.027 → 0.111] |
| common neighbours | 0.22 → 0.20 | 0.46 → 0.52 | 0.42 → 0.66 | 0.50 → 0.71 | 0.34 → 0.52 |
| 2-hop count | 1.84 → 1.82 | 32.4 → 32.5 | 668 → 753 | 14.1 → 16.2 | 4.44 → 5.99 |
| 3-hop, 10k budget | 37.9 → 40.9 | 48.0 → 50.8 | 34.8 → 72.6 | 35.7 → 41.0 | 35.9 → 46.5 |
| bidirectional BFS | 128 → 161 | 341 → 450 | 2,982 → 3,361 | 1,653 → 2,334 | 465 → 562 |

- Neighbour lookups cost +0.02–0.11 µs per call on the batch means (2.5–4.2×); bidirectional
  BFS p50 1.13–1.41×; 2-hop count p50 0.99–1.35×; 3-hop with a 10k budget p50 1.06–1.30×
  except twitter-sym, 2.1× at p50 and 2.7× at p95 (twitter-2010 directed: 3.1× at p95).
  The budgeted query stops after 10,000 nodes, but the compressed path decodes a hub's whole
  list before it looks at the first neighbour; the format allows prefix decode, the query
  does not use it (not fixed, not measured).
- Tails are in summary § 4 (p95 / p99); for example twitter-sym common neighbours p99 is
  1,397 → 1,556 µs.
- With the graph mmapped (4 KiB pages) both sides are slower (neighbour batch means
  +16–43%); bidirectional BFS p50 is then 1.09–1.50×. C equals A on lj and orkut, the only
  graphs where it was run.
- Idle, the shortest path between two nodes takes 0.13–0.14 ms on lj on raw arrays and
  0.16–0.17 ms on the default storage (heap / mmap), 0.44–0.47 and 0.56–0.59 ms on
  twitter-2010 directed.

**Memory** (kernels at 16 threads; `rss_anon_gb` = anonymous memory at the end of the
kernel, i.e. graph plus leftovers; `peak_rss_gb` = high-water mark, anonymous plus
file-backed):

| graph, kernel | A raw: anon / peak | B compressed: anon / peak | C decoded: anon / peak |
|---|---|---|---|
| orkut, PageRank f32 | 0.91 / 0.94 GiB | 0.46 / 0.50 | 0.90 / 1.35 |
| twitter-sym, PageRank f32 | 9.27 / 9.74 | 4.77 / 5.24 | 9.27 / 13.79 |
| Friendster, PageRank f32 | 13.95 / 14.68 | 9.76 / 10.50 | 13.98 / 23.71 (BFS row) |
| Friendster, triangles (forward lists raw) | 13.95 / 21.41 | 9.76 / 17.23 | – |

C's peak includes the compressed bytes held in the heap during decode; with the store
mmapped they are file-backed and evictable. Leiden's sampled anonymous peak at 16 threads,
graph in heap: twitter-sym 25.0 GiB (A) vs 21.0 (B); Friendster 37.0 vs 34.3. Subtracting
the graph (arithmetic), Leiden's own memory on Friendster is 23.1–24.5 GiB in both: only the
level-0 graph shrinks. **Compression does not bring Friendster Leiden under 16 GiB.**

**Under a hard memory limit** (summary § 8, `capped.jsonl`; 16 threads, files evicted
first, plain mmap without pre-fault, `MemorySwapMax=0`, best of 2 with the first repetition
cold; PageRank f32 with 5 iterations including the initial degree pass):

| case | A raw | B compressed | C decoded |
|---|---|---|---|
| twitter-sym, 8 GB: PageRank | 7.22 s | 3.84 s (1.9× faster) | killed (exit 137) |
| twitter-sym, 8 GB: WCC / BFS | 0.275 / 0.193 s | 0.210 / 0.287 s | – |
| Friendster, 12 GB: PageRank | 19.4 s | 16.3 s (1.2× faster) | killed (exit 137) |
| Friendster, 12 GB: WCC / BFS | 2.63 / 4.59 s | 0.491 / 0.519 s (5.4× / 8.8× faster) | – |
| Friendster, 2 GB: PageRank (1 rep) | 22.1 s (4.42 s/iter) | 21.6 s (4.31 s/iter) | – |

- When the raw arrays do not fit and the compressed store does, compressed is faster for
  PageRank (1.2–1.9×) and for WCC and BFS on Friendster (5.4–8.8×). BFS on twitter-sym is
  the exception (1.5× slower compressed): it touches a small share of the entries, which
  stay resident even for raw.
- **Under 2 GB, plain mmap gives the format no gain on Friendster.** 4.31 s/iteration
  against 3.19 in RAM on the same format is 1.35×; raw 4.42 against 3.36 is 1.32×
  (arithmetic: a 5-iteration capped run that includes the initial degree pass, against
  20 iterations in RAM). For comparison, the B1 raw-format streaming reader gave 3.64 against
  3.37 s/iteration (1.08×, 32 threads, idle re-runs above). There is no streaming reader for
  the compressed format yet.

**Ablations that explain the result** (summary §§ 6, 7 and 9; `ablation.jsonl`,
`scan.jsonl`, `perf.jsonl`):

- *Batch vs list-by-list decode is the largest effect.* Decoding list by list, interleaved
  with the kernel body, makes orkut PageRank f32 4.92 s at 1 thread and 0.367 s at 16;
  decoding a batch of 8,192 entries first gives 3.65 and 0.266 (1.35× and 1.38×). On lj
  (short lists) batching does not help at 1 thread (1.86 s list by list vs 1.95). perf,
  orkut PageRank f32, 1 thread, whole process: raw 45 G instructions at IPC 0.71 (63 G
  cycles); batched 125 G at IPC 2.08 (60 G cycles); list by list 117 G at IPC 1.40 (83 G
  cycles). Raw waits on memory and the decode fits in the wait; interleaving costs cycles
  without adding instructions, which is consistent with fewer overlapping cache misses
  (not measured directly).
- *Why 16-thread PageRank is faster compressed on orkut:* raw runs at IPC 0.49 and 309 G
  cycles, compressed at 1.97 and 212 G. A raw scan tops out at 12.4–13.7 G entries/s on
  the four graphs at 16 threads (49.6–54.9 GB/s of 4-byte targets, arithmetic); the
  compressed scan with the default decoder and 8,192-entry batches reaches 14.9–27.5 (lj
  14.9, Friendster 17.9, twitter-sym 26.4, orkut 27.5). At 1 thread the compressed scan is
  2.2–2.5× slower. On twitter-sym PageRank the gathers dominate (IPC 0.13 raw vs 0.37,
  cycles within 5%), so it is a tie.
- *Per-node vs block offsets.* With a block directory only, random access to one list costs
  10–33 µs instead of 87–217 ns (1 thread; raw 33–62 ns); orkut BFS at 16 threads takes
  2.24 s instead of 0.028; bidirectional BFS p50 31.8 ms. Per-node offsets are required.
- *Entry-offset index.* An Elias–Fano index of entry offsets (0.11–0.35 bits/edge,
  `verify.jsonl` `degree_index.bits_per_edge`) makes degree a lookup without touching the
  data: BFS at 16 threads becomes 1.01× of raw on orkut, 0.99× on twitter-sym, 1.07× on lj,
  1.22× on Friendster; at 1 thread it stays 1.56–1.75× (lj, orkut). It did not measurably
  improve bidirectional BFS p50 (one repetition, mixed sign). Keeping the degree in the
  traversal state instead (`bfs-ds`): 1.15–1.25× at 16 threads on lj, orkut and
  twitter-sym, no gain on Friendster (1.43×).
- *BFS is instruction-bound:* orkut, 1 thread, 38.8 → 76.0 G instructions and 21.5 →
  43.6 G cycles at about the same IPC. Likely causes (not separated): whole-list decode where
  raw stops at the first hit, a degree lookup per discovered node (17–64 ns against 4–6 ns
  raw), a header pass to find the source.
- *WCC slowdown grows with graph size* (1 thread: 0.96× on lj and orkut, 1.25× twitter-sym,
  1.57× Friendster), although perf on orkut shows 2.8× the instructions in fewer cycles.
  Hypothesis, not tested: the per-node prefix decode reduces overlapping misses once the
  label array leaves the cache. A batched prefix decode is written and unit-tested in the
  lab but was not measured.
- *Decoder:* lab C's decoder is 1.17–1.25× slower than the default on PageRank at 1 thread
  (orkut f32 4.57 vs 3.65 s). On short-list graphs a per-list cost remains: lj PageRank at
  1 thread is 1.24–1.40× with every decoder and batch size (one loop exit per list is the
  likely cause, not proven).
- k-core, triangles and Leiden at 1 thread on orkut / lj: +20–32% instructions, cycles
  within −4% to +8%. perf counters are whole-process user space (graph loading included),
  multiplexed.

**The planner rule that follows.** Let R = raw bytes (4 per entry + 8 per node), Z =
compressed bytes (0.49–0.70 R measured), S = the kernel's own state, M = the budget.

1. **Z + S ≤ M: run on the compressed store, decoded on the fly, for every kernel.**
   Scans (PageRank, k-core initialisation, the triangle build, WCC sampling) decode batches
   and never decode to raw; heavy random access (k-core peeling, Leiden, triangles on
   derived forward lists) runs on the fly at 0.88–1.09× of raw at 16 threads; a single
   traversal (BFS 0.018–0.51 s at 16 threads compressed) is cheaper than decoding the whole
   graph (0.02–0.99 s).
2. **Build the raw decode cache only if all three hold:** R + S ≤ M with headroom; the
   session repeats traversals or point lookups; the expected saving exceeds the decode time.
   Break-even, arithmetic on measured values (16-thread warm decode time ÷ per-operation
   difference B − A; BFS at 16 threads, queries at 1 thread):
   - orkut (decode 0.060–0.061 s, +0.90 GiB): about 7 BFS runs; about 280 shortest-path
     queries from the mean difference (560 from the p50 difference); about 0.85 M neighbour
     lookups;
   - twitter-sym (0.60–0.68 s, +9.27 GiB): 6–7 BFS runs; 400–460 shortest-path queries
     (1,600–1,800 from p50); 5.6–6.4 M neighbour lookups;
   - Friendster (0.92–0.99 s, +13.95 GiB): about 6 BFS runs; 900–970 shortest-path queries
     (1,350–1,450 from p50); 8.2–8.7 M neighbour lookups.
   These are not measured sessions; a session's total cost follows the mean, not the p50.
   Never build it for PageRank, WCC, k-core, triangles or Leiden.
3. **Z + S ≤ M < R + S: compressed only.** Raw through mmap is slower for scanning kernels
   (PageRank 1.2–1.9×, WCC 1.3–5.4×) and decoding to raw is killed.
4. **Z + S > M: out of core.** Plain mmap of the compressed store gains nothing on
   Friendster under 2 GB; this regime needs a streaming reader for the format (#46).

**Conditions on D4** (ADR 0004): per-node Elias–Fano offsets, not block offsets; batched
sequential decode in the edge-map framework; prefix decode for budgeted queries; an
Elias–Fano index of entry offsets in the format; the raw decode cache as a planner tier
with the break-even above. Open format question: per-list framing costs short-list graphs
1.24–1.40× on single-thread PageRank; a block-framed stream would remove the per-list branch
but complicate random access (not tested).

**Not measured, not trusted.**

- *Not run* (time): PageRank, k-core and triangles at 1 thread on Friendster; triangles at
  1 thread on twitter-sym; Leiden at 1 thread on twitter-sym and Friendster; deterministic
  Leiden on Friendster; configuration C on the big graphs beyond the rows shown (and for
  point queries beyond lj and orkut).
- *Not measured:* point-query throughput at 16 threads (ADR 0004's criterion names 16
  threads; this run measured single-thread latency, as the issue asked); a streaming or
  semi-external reader on the compressed format; ingest writing the format directly, with
  or without `fsync`; an encoder under a memory budget; weighted or directed kernels; web
  graphs; prefix decode for budgeted queries; batched prefix decode for WCC.
- *Not trusted:* k-core at 16 threads on orkut (0.22–0.36 s in A, B and C alike; the
  ratio is meaningless); the one-repetition rows (Friendster and twitter-sym triangles,
  twitter-sym PageRank at 1 thread, twitter-sym deterministic and Friendster Leiden,
  Friendster under 2 GB); Friendster encode at 1 thread (74.7 s with `busy_cpus_before`
  0.36; a discarded run gave 49.2 s); point-query p50 below ~0.2 µs (use the batch means);
  the entry-offset-index point-query rows (one repetition, mixed sign).
- *Machine state:* one foreign single-threaded job skewed these kernels by 10–42% in a
  discarded attempt; 1-thread and bandwidth-bound runs are the most sensitive, which is why
  the gate counts foreign CPU before every row.
- Every "why" beyond the perf counters is inference.

## M1-01 — tracer bullet: Friendster text to map under 16 GiB (#15; 2026-11-25, idle machine)

**Question.** Does the chain text → CSR → communities → hierarchy → layout → tiles → viewer
complete on com-Friendster (65.6 M nodes, 1.8 B edges) inside a hard 16 GiB limit, and what
does each stage cost? **No.** Under 56 GiB it completes, but only with the map-shaped
Louvain hierarchy: the map-shaped Leiden is killed at 56 GiB too.

**Setup.** One official run of `bench/specs/tracer.toml` through the benchmark harness,
revision `3e764d1`, 2026-11-25 12:26–14:36 UTC:
`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` ("row N" below is line N of
that file). The gate saw a 1-minute load of 0.85 and 59.2 GiB available; other processes
used at most 0.40 cores during any row (gate 0.5); the per-row load average of 12–32 at
start is the run's own previous stage. Verification, screenshots, the lab tools' own
reports and the failure logs are in `lab/m1-01-tracer/results/`; bare file names below are
relative to it.

- *Tools:* the lab binaries glued by `lab/m1-01-tracer/tracer.py`, no new algorithms: glab
  ingest; b2 Leiden or Louvain, async, 1 iteration, f32 quotient weights, `--lowmem
  --no-populate`, CSR memory-mapped; gmap layout and tiles; the lab viewer in headless
  Chromium on the RTX 4090 through ANGLE, 1600×1000.
- *Conditions:* 32 threads (viewer 1); inputs evicted from the page cache before each stage;
  uncompressed CSR (`glab-csr-v0`); no `fsync`. Each stage in its own scope with
  `MemoryMax=16G` and again with `56G`, `MemorySwapMax=0`.
- *Map-shaped* is experiment E's hierarchy setting, `--cap 64 --max-passes 20 --min-ratio 3`
  (fan-out cap 64, levels kept only if they shrink 3×); *plain* is `--min-ratio 3` alone.
- *Numbers:* times are the harness's primary metric (the tool's own seconds), median of the
  repetitions shown. "anon" is the sampled anonymous peak of the scope. The cgroup peak
  (`memory.peak`, page cache included) reads 16.0 or 56.0 GiB whenever a stage sits at its
  limit, so it is given only where it is lower. Memory in GiB, disk in GB (10^9).

| stage | 16 GiB | 56 GiB |
|---|---|---|
| ingest, text | budget 8 GB: ok, 52.3 s (51.9–59.1, 3 reps after a warm-up), anon 3.6 GiB, disk peak 40.4 GB, CSR 15.5 GB (rows 1–3) | budget 32 GB: ok, 30.7 s (27.5–32.6), cgroup peak 28.0 GiB, anon 13.2 GiB, disk peak 30.1 GB (rows 4–6) |
| ingest from .gz (pigz pipe) | budget 8 GB: ok, 103.8 s (102.7–105.0, 2 reps), anon 3.6 GiB (rows 7–8) | not run |
| Leiden, map-shaped | killed at 460.5 s in pass-0 aggregation, anon 15.9 GiB, 1.80 TB read (row 9) | killed in the warm-up at 117.3 s, after pass 4, anon 55.8 GiB (row 10 only says "failed"; the measurement is in `logs/communities-leiden-56G.warmup1.measure.json`) |
| Louvain, map-shaped | killed at 648.7 and 648.2 s (2 of 2) in pass-0 aggregation, anon 15.9 GiB, 2.72–2.74 TB read (rows 11–12) | ok, 110.7 s (110.6–110.8, 2 reps), anon 42.9 GiB (b2's own 28.3 GiB before the hierarchy write), 42.5 GB written, Q 0.5771 and 0.5757 (80 and 52 top communities; the chain used the second) (rows 13–14) |
| Louvain, plain | ok, 115.1 s, anon 4.2 GiB, 0.46 GB written, Q 0.6038, 33,306 communities, fan-out up to 18.7 M (row 15) | not run |
| layout, plain-Louvain hierarchy | killed: graph in heap at 27.0 s (row 16); graph mmapped at 74.1 s, during depth 3 (row 17) | killed at 93.2 and 89.1 s during depth 3, anon 55.8 GiB (rows 20–21) |
| layout, map-shaped Louvain hierarchy | graph in heap: killed at 15.7 s (row 18); graph mmapped: ok, 405.1 s, anon 3.7 GiB, 733 GB read, 1.99 GB written (row 19) | ok, 69.6 s (69.5–69.8, 2 reps), cgroup peak 33.3 GiB, anon 17.7 GiB, 1.99 GB (rows 22–23) |
| tiles, same hierarchy | graph in heap: killed at 13.2 s (row 26); graph mmapped: cut by the 60-minute timeout after zoom levels 0–10 of 0–11, anon 13.9 GiB (row 27) | ok, 244.5 s (1 rep after a warm-up), cgroup peak 51.7 GiB, anon 29.1 GiB, 6.84 GB file, 847,925 tiles, zmax 11 (row 29) |
| tiles and viewer, plain-Louvain chain | no input, no layout exists (rows 24, 25, 30) | no input (row 28) |
| viewer, on the 56 GiB tile file | ok, 18.2 s, anon 1.9 GiB; first view 163,184 B in 18 requests; session 5,624,158 B in 577 requests; cold search 289,056 B in 2 requests (row 31) | – |

Verification (`lab/m1-01-tracer/results/verify_friendster.json`): the three ingest outputs
(8 GB text, 8 GB gzip, 32 GB text) are byte-identical to the reference CSR
`data/csr/friendster`. On Friendster, byte identity between the graph in the heap and
memory-mapped is shown for the layout only (16 GiB mmap against 56 GiB heap, same SHA-256 of
positions and tree nodes). Only one tile file exists (the 16 GiB one did not finish), so
the file's "identical across variants" for tiles compared it with nothing. The independent
review of the lab PR (#138) found layout and tile file byte-identical between heap and mmap
on lj (map-shaped Louvain, 8 threads), not on Friendster.

- **The 16 GiB claim fails at community detection.** Leiden and the map-shaped Louvain are
  killed in the first aggregation, after the local moves (and Leiden's refinement) have
  finished: Leiden's pass 0 took 65.6 M nodes to 20.9 M communities in 37.8 s of moves and
  6.3 s of refinement, Louvain's to 10.5 M
  (`logs/communities-leiden-16g__graph-friendster__t32.rep1.err.tail`,
  `logs/communities-louvain__graph-friendster__limit-16G__t32.rep1.err.tail`). Anonymous
  memory reached the limit while the 15 GB mapped graph was being re-read: 1.80 TB read by
  Leiden, 2.72–2.74 TB by Louvain (rows 9, 11–12). Leiden's level-1 quotient graph alone is
  about 22 GB (arithmetic: 2.77 B entries × 8 B, from the 56 GiB warm-up log).
  *Addressed in the lab, 2026-11-25 (§ M1-02, #16; Friendster, async, 1 iteration, 16
  threads, compressed level 0, not b2's raw CSR at 32 threads):* with sequential
  aggregation and quotient graphs spilled as compressed stores, Leiden completes under
  16 GiB in 87.4 s (1.17× the same algorithm in RAM) and map-shaped Leiden under 16 GiB in
  164.1 s (Q 0.5900, 27 top communities). The production code is #60; layout (#67) and
  tiles (#69) remain the blockers of this chain.
- **The partition that fits cannot be drawn.** Plain Louvain fits (4.2 GiB anonymous;
  aggregation 74.6 s of b2's 104.3 s, `b2_louvain_plain_16G.json`), but its hierarchy has a
  parent with 18,723,769 children, the largest community 19.8 M nodes and 62 disconnected
  communities. The lab layout builds an unbounded map over the sibling pairs of any parent
  with more than 384 children (code, per the review of #138), and it was killed during
  depth 3 even at 56 GiB (rows 20–21); that the 18.7 M-child parent is the one it died on
  is inference (the log shows depths 0–2 completing).
- **Capped coarsening shrinks slowly, and the prototype keeps every level.** Under the
  fan-out cap, level 1 has 10.5 M communities against plain Louvain's 1.16 M, and the capped
  Leiden's pass 3 still has 1.42 B entries (pass 4: 0.70 B; warm-up log). With `--out-hier`,
  b2 keeps every level's quotient graph in memory (`keep_graphs` in
  `lab/b2-leiden/src/main.rs`); that is what kills the capped Leiden at 56 GiB: about
  51.5 GiB of quotient graphs (arithmetic: 2.77 + 2.03 + 1.42 + 0.70 B entries × 8 B; not
  tested without `--out-hier`). The capped Louvain completes at 56 GiB with a scope
  anonymous peak of 42.9 GiB (rows 13–14), but b2's own anonymous peak, recorded before it
  writes the hierarchy, is 28.3 GiB (`b2_louvain_capped_56G.json`); the difference is most
  likely the writer's transient f64 copy of the quotient weights (the level-1 quotient has
  2.14 B entries, about 16 GiB as f64; inference from the code and these numbers, by the
  review of #138). Plain Louvain: 4.2 GiB at 16 GiB (row 15; b2's own figure 3.7 GiB). The
  capped Louvain writes 42.5 GB of hierarchy, 2.7× the CSR; about two thirds of that is the
  f64 weights of the quotient graphs (arithmetic: 3.49 B quotient entries over passes 1–8
  × 8 B = 27.9 GB), which neither the layout nor the tiles read (gmap reads only the
  `level_k.u32` parent maps). **No Leiden map exists at Friendster scale yet.**
- **With a map-shaped hierarchy, layout fits 16 GiB** if the graph is memory-mapped rather
  than loaded into the heap (the graph alone is 13.9 GiB of heap, arithmetic: 3.61 B
  4-byte targets and 65.6 M 8-byte offsets): 3.7 GiB anonymous, at 5.8× the time (405.1
  against 69.6 s at 56 GiB, arithmetic), re-reading 733 GB. These mmap figures are with
  the kernel's default readahead: gmap makes no `madvise` call, and `MADV_RANDOM` was not
  tried. The map-shaped hierarchy (Q 0.5757; levels 10,511,685 / 1,332,441 / 239,714 /
  69,440 / 13,413 / 52; fan-out ≤ 64 on the four lower levels, then up to 1,094 and 6,478;
  largest community 17.7 M nodes; 25 disconnected) gives 11,346,253 tree nodes, 731
  derived regions and depth 8 (`layout_unl_56G.json`). It is the only map produced.
- **The tile builder does not fit 16 GiB.** It keeps every tile in RAM (gzip bytes, about
  2.55 GB through zoom 10 by the log's per-level sizes) and, per zoom level, a working state
  (candidate sets, per-tile hash maps, positions, order) that is most of the rest: 13.9 GiB
  anonymous together, leaving about 2 GiB of the 16 GiB for page cache. Each zoom level
  re-read the graph, 68–211 s per level for zoom 0–4 and 278–491 s for zoom 5–10, against
  7.1–32.5 s per level at 56 GiB
  (`logs/tiles-16g__chain-unl__graph-friendster__load-mmap__t32.rep1.err.tail`,
  `tiles_unl_56G.json`). At 56 GiB: 5.79 GB of gzip tiles plus a 1.05 GB search index;
  tile median 6.8 KB gzip, maximum 32.8 KB; 7.4% of edge weight shown at the deepest zoom.
- **Slowdown 16 GiB against 56 GiB, same input** (arithmetic on the medians): ingest 1.70×
  (52.3 against 30.7 s), not like for like: the budgets differ, 8 GB and 32 GB; layout 5.8×
  (mmap against heap); tiles at least 14.7× and unfinished.
- **Unlimited, the chain takes 455.5 s of stage time** (30.7 + 110.7 + 69.6 + 244.5,
  arithmetic on the medians, Louvain hierarchy, viewer excluded) and writes 66.9 GB: CSR
  15.5, hierarchy 42.5, layout 2.0, tiles 6.84 (rows 6, 14, 23, 29), plus the 32.4 GB text
  input (`text_bytes` in `lab/b1-core/results/ingest.jsonl`). The ingest work directory
  peaks at 30.1 GB with a 32 GB budget and 40.4 GB with 8 GB, the 15.5 GB output included.
  The whole store of the run (three CSR variants, two hierarchies, two layouts, one tile
  file) was 100.3 GB.
- **The viewer reads well** (screenshots at zoom 1.9 / 4.52 / 7.14 / 9.37 / 11.6 and after a
  search, `lab/m1-01-tracer/results/viewer/`; the experimenter's visual judgment):
  continents of 1–18 M nodes with exact counts, node-level edges inside 64-node
  communities. Fly-to steps cost 0.93–1.47 MB; a pan at node level 0 B. Frames 16.67 ms
  mean, none over 33 ms: vsync-capped, so this shows no dropped frames, not headroom
  (`viewer/viewer.json`). The first view, 163,184 B, is under DESIGN § 6's 250 KB. The
  header shows the store directory "csr.16G-b8" as the graph name (cosmetic).
- **Against the § E extrapolations** (layout 2–4 min and ~20 GB; 9–13 GB of tiles): the
  layout took 69.6 s with 17.7 GiB anonymous and the tiles 244.5 s for a 6.84 GB file, on
  this hierarchy, with 56 GiB. § E is corrected in place.
- **The § B2 Louvain figure was not reproduced:** plain Louvain under 16 GiB gave Q 0.6038
  here against 0.4647 (one run each, not diagnosed; § B2 is annotated). This run's passes
  1–3 converged in 6, 3 and 7 iterations; in the § B2 run passes 1–4 each hit the
  20-iteration cap (`b2_louvain_plain_16G.json`,
  `lab/b2-leiden/results/big/friendster_louvain_semiext16g.json`).
- **The two map-shaped Louvain repetitions differ** (async): Q 0.5771 with 80 top
  communities (row 13) and 0.5757 with 52 (row 14). The map used the second.

**Harness defects** (follow-up to #4; confirmed from the harness code by the review of
#138): a failed warm-up writes a generic "failed" row without its measurement (row 10), so
the Leiden 56 GiB peaks exist only in a log file; on a timeout the harness kills every
process of the scope at once, and its `/proc/self/io` counts only descendants it has
reaped, so row 27 records `read_bytes` 86,016, its own page-fault reads count as others'
storage traffic (3,211 MB/s) and the row is wrongly marked disturbed.

**Not measured.** A Leiden hierarchy under any limit (plain Leiden with hierarchy output,
which fits 34 GiB without it, was not in the spec); layout and tiles on a Leiden hierarchy;
ingest under 16 GiB with a budget other than 8 GB; tiles under 16 GiB to completion;
deterministic or multi-iteration Leiden; the compressed default storage (the lab binaries
read raw CSR); `fsync`; frame-time headroom (vsync-capped); a viewer without a GPU.
Repetitions: 1 for the expected-OOM cases and for tiles, 2 for communities and layout.

---

## M1-02 — out-of-core Leiden (#16; 2026-11-25, idle machine)

Code `lab/m1-02-ooc-leiden`; spec `bench/specs/ooc-leiden.toml`; official runs
`bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl` (25 rows, revision
e501e2c) and `…20261125T174959Z-b61036e59740.jsonl` (the same-algorithm in-RAM reference,
6 rows, revision b61036e, which differs from e501e2c only by the review fixes), each with
its `.program.jsonl` (per-pass phases, bytes read and written, accountant report, hashes);
0 disturbed rows. Exploratory records in `lab/m1-02-ooc-leiden/results/`. Times below are
medians of the counted repetitions (1–3 per case).

**Setup.** Friendster, async Leiden, 1 iteration, 16 threads, f64 weights, level 0 in the
D4 compressed store; kernel time only (no open, no evaluation). Out of core = cold private
copy, mmap without pre-fault, sequential aggregation into range-partitioned buckets (RAM
while the accountant allows, else files), quotient graphs as compressed stores on disk,
every large allocation reserved in an accountant stub (budget 0.80 × limit − 64 MiB).
Reference: the same algorithm in RAM (nothing spilled, warm, `--ooc --budget-gb 40` under
48 GiB): 74.8 s (74.4–75.5, 3 repetitions), Q 0.6038 (0.6034–0.6055), anonymous peak
29.0 GiB, 27.3 GB of buckets held in RAM. The member-order `--lowmem` aggregation takes
100.1 s (2 repetitions); M1-13's default member-order aggregation took 82.0 s with no
limit (`lab/m1-13-compressed/results/leiden_big.jsonl`) and is OOM-killed under 48 GiB
(exit 137, `lab/m1-02-ooc-leiden/results/default_agg_oom.jsonl`, exploratory).

**Results**, against the same-algorithm reference (in brackets: against `--lowmem` 100.1 s
and against M1-13's 82.0 s):
- 34 GiB: 77.1 s (1.03×; 0.77×, 0.94×);
- 16 GiB: 87.4 s (1.17×; 0.87×, 1.07×), anonymous peak 8.9 GiB;
- 8 GiB: 175.3 s (2.34×; 1.75×, 2.14×), anonymous peak 4.8 GiB;
- 5 GiB: 215.8 s (2.88×; 2.16×, 2.63×), 1 repetition;
- 4 GiB: 260.9 s (3.49×; 2.61×, 3.18×), anonymous peak 2.7 GiB, 1 repetition.

0 disconnected communities in every row. Async Q 0.598–0.607 with block order, in RAM and
out of core alike (two of the eight out-of-core block-order repetitions are below 0.60:
0.5981 at 16 GiB, 0.5979 at 8 GiB). Deterministic Leiden gives the same partition in RAM
(`--lowmem`, 107.3 s) and under 16 GiB (97.0 s): hash `2a12e5882874c38d`, Q 0.6055, 44,641
communities. On lj and orkut, deterministic, the hashes in RAM and out of core at 0.25,
0.3 and 0.6 GiB budgets are equal (`44ae14c7774277eb`, `ca792d246e7f461f`), with
`--check-agg` confirming every level's quotient graph equal to the member-order aggregation
(6 of 6 levels), and lj also at 1 and 4 threads (`results/determinism.jsonl`, exploratory).
Map-shaped orkut gives the same hash and byte-identical level files in RAM and out of core
(`results/map_identity.jsonl`, exploratory).

**Costs.** Level-0 aggregation takes 1.62× its in-RAM time at 16 GiB (26.3 s against
16.2 s) and 1.94–2.01× at 8–4 GiB, because 20–27 GB of buckets go to files and are read
back (`bucket_disk_bytes`). Below 16 GiB, level-0 local moving with ADR 0015's
block-permuted order re-reads the store (10.4 GB) 6.0×, 8.7× and 12.8× per sweep at 8, 5
and 4 GiB (103.4, 121.0 and 150.2 s of local moving against 35.2 s; 406, 545 and 799 GB
read). At 16 and 34 GiB level 0 reads about nothing: the store read by the strengths pass
stays cached. **Smallest budget within 2.5× of the same algorithm in RAM: 8 GiB** (5 GiB
only against `--lowmem`). The accountant's reservations exceed the measured anonymous peak
(the gap is reserved headroom); no row exceeded its limit; demotion of RAM buckets to files
and waits on memory occur in no official row (`demoted_bytes` and `mem_waits` 0), only in
unit tests and one reviewer reproduction on lj.

Map-shaped Leiden (cap 64, compaction 3), which b2 could not run at 56 GiB (§ M1-01), runs
under 16 GiB in 164.1 s, Q 0.5900, 27 top communities, anonymous peak 10.9 GiB; no in-RAM
reference exists. orkut under 447 MiB (50% of its raw edge array): 6.01 s, 3.10× the same
algorithm in RAM (1.94 s) and 3.42× `--lowmem` (1.76 s); Q 0.6799; anonymous peak 0.23 GiB
(the old lab: 5.6× near the edge-array size, 51× or OOM below it, § B2).

**Survey ideas.** A seeded relabelling plus storage-order visits (L1) reads one store per
sweep and runs Friendster in 95.4 s under 8 GiB and 107.2 s under 4 GiB (1.28× and 1.43×
the block-order same-algorithm reference) with Q 0.611 (0.6097–0.6118); its own in-RAM run
(`--lowmem`) takes 95.0 s. Its costs are compression (+5% on Friendster, +16% orkut, +26%
lj; `results/stores.jsonl`) and, in exploratory runs (`results/orders.jsonl`), in-RAM time
1.21–1.22× on lj and orkut and modularity −0.3 points on lj and −1.7 on orkut. Id order on
Friendster's original ids is unstable (Q 0.558–0.612, 3 exploratory runs,
`results/friendster_idorder.jsonl`). L1's comparison is confounded (ids and order change
together; one graph, one seed; deterministic mode not measured); it is under study in
`plan/issues/m1-14-relabel-at-ingest.md` (#146), and ADR 0015 is unchanged. Windowed permutation
(L2) loses the quality of block order unless the window covers the graph; at level 0, L3
finds no inactive blocks (`results/orders.jsonl`); never building level 1 (L4) would cost
about 3× the spill path (arithmetic on official numbers, not measured). L5 not tested.

**Not measured.** twitter-sym and other graph families; more than one iteration;
deterministic mode below 16 GiB; a same-algorithm in-RAM reference for L1 or deterministic
mode; thread counts other than 16 on Friendster; more than 2 async repetitions per budget;
weighted inputs (exactness across budgets relies on integer weights); a disk budget (a full
disk is refused with exit 5, not planned for).
