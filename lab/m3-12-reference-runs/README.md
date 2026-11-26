# m3-12 — reference runs: Everygraph against the best-known kernel codes (#55, #265)

Question: on this machine, how do Everygraph's production kernels compare with the best
tools — GAP, GBBS, ConnectIt, Parallel-KCore, PASGAL, the COST line, NetworKit,
python-igraph, GVE-Leiden — on the same graphs, inputs and thread counts?

**There are no speed ratios here yet.** A ratio between tools is a result only from an
official run on an idle machine, with intervals (CLAUDE.md § 3; #265). The exploratory
rounds of this branch (`results/rounds.jsonl`, `results/friendster.jsonl`, made by
`interleave.py` with the spec's first commands on a machine at load 5–160) showed why:
the same cross-tool ratio changed sign between rounds (#265 quotes one). They are kept
as raw rows and are not results.

What this directory holds instead, all independent of the machine's load:

- **validated outputs**: every tool's result checked against Everygraph's (below);
- **memory peaks** of one run of each orkut, roadnet-ca, web-google and Friendster case
  (below);
- the method, in `bench/specs/references.toml`, and the official commands.

## Official commands

The orkut window (the review's experiment F4: every reference tool, validated, 6
repetitions; the intervals come from the same rows once #194 lands), validation first:

```sh
everygraph-bench run references --official --only validate/on=orkut --data /home/tux/graphlab/data
everygraph-bench run references --official --only graph=orkut/t1 --only leiden-gve/graph=orkut --repetitions 6 --data /home/tux/graphlab/data
```

The whole spec (every graph):

```sh
everygraph-bench run references --official --data /home/tux/graphlab/data
```

Duration, an estimate from the exploratory pass (`results/once.jsonl`, a loaded
machine): one repetition of the 51 orkut cases took 32 min of wall time, so the window
takes about 3 h at 6 repetitions, plus 8 min of validation. The whole spec takes about
8 h at 3 repetitions (an extrapolation from orkut, Friendster and the small graphs;
twitter-sym and lj were not timed in that pass).

## Method

`bench/specs/references.toml` holds every rule; in short:

- **Same input.** The registry's CSR (`csr/<graph>`: symmetric, sorted, no self-loops,
  input order), converted once to each tool's format by `bench/runners/run.py convert`,
  with the same node ids. No graph has isolated vertices (counted from `offsets.u64`):

  | graph | family | n | undirected edges | isolated |
  |---|---|---|---|---|
  | lj | social | 3,997,962 | 34,681,189 | 0 |
  | orkut | social | 3,072,441 | 117,185,083 | 0 |
  | roadnet-ca | road | 1,965,206 | 2,766,607 | 0 |
  | web-google | web | 875,713 | 4,322,051 | 0 |
  | twitter-sym | social | 41,652,230 | 1,202,513,046 | 0 |
  | friendster | social | 65,608,366 | 1,806,067,135 | 0 |

- **Same work.** PageRank f64, damping 0.85, 20 iterations, tolerance 0. BFS and SSSP
  from the same 64 seeded non-zero-degree sources, GAP's own (`gap_sources.cc` prints
  SourcePicker's list). BFS returns parents (GAP, GBBS, Everygraph) or distances
  (PASGAL, NetworKit, igraph). SSSP on the same per-entry costs (integers 1 to 255), in
  f64 (Everygraph, GAP with `WeightT = double`) and in integers (GAP int32, PASGAL
  uint32): integer sums are exact in both, so the distances are equal. Leiden: each
  tool's defaults; GVE-Leiden in f32 and f64.
- **Same statistic.** Every process runs a whole-graph kernel 5 times and reports the
  median; a search once per source and reports the mean; Leiden once. The harness
  repeats the process.
- **Same flags.** Every C++ tool and Everygraph without `-march=native` /
  `target-cpu=native` (`bench/references/build.sh generic`; GBBS's makefile flag
  removed), each with its own optimization level (`-O3`; Rust release with fat LTO).
  The `native` flavour is built too, not run.
- **Same placement.** Every process runs under `bench/references/pin.sh`: the 16
  physical cores (`taskset -c 0-15`), OpenMP threads bound one per core.
- **Storage stated.** GBBS maps its graph file (`-b`); the lab Leiden maps the CSR;
  every other tool reads the graph into the heap. Load time is apart from kernel time
  where the tool reports it.

## Validated outputs

`bench/runners/run.py validate` runs each tool once (16 threads) with its result written
out and compares it with Everygraph's: the partition into components (labels renumbered
by first appearance), every core number, every distance from the first of the 64
sources (a parent array is turned into depths; on a sample of 20,000 nodes each parent
must be a neighbour), PageRank by L1 distance (at most 1e-6), and triangle counts
between the tools (Everygraph has no triangle kernel). A tool whose check fails is not
reported. Results: `results/validation-<graph>.json` (one line each, as
`run.py validate` prints it), from the spec's `validate` cases (rows:
`results/once-validate.jsonl`).

| kernel | tool | lj | orkut | roadnet-ca | web-google | twitter-sym | friendster |
|---|---|---|---|---|---|---|---|
| bfs | gap | yes | yes | yes | yes | yes | yes |
| bfs | gbbs | yes | yes | yes | yes | yes | yes |
| bfs | igraph | yes | yes | yes | yes | — | — |
| bfs | networkit | yes | yes | yes | yes | — | — |
| bfs | pasgal | yes | yes | yes | yes | yes | yes |
| cc | connectit | yes | yes | yes | yes | yes | yes |
| cc | cost | yes | yes | yes | yes | yes | yes |
| cc | gap | yes | yes | yes | yes | yes | yes |
| cc | gbbs | yes | yes | yes | yes | yes | yes |
| cc | igraph | yes | yes | yes | yes | — | — |
| cc | networkit | yes | yes | yes | yes | — | — |
| kcore | gbbs | yes | yes | yes | yes | yes | yes |
| kcore | igraph | yes | yes | yes | yes | — | — |
| kcore | networkit | yes | yes | yes | yes | — | — |
| kcore | pkc | yes | yes | yes | yes | yes | yes |
| pr64 | cost | yes | yes | yes | yes | yes | — |
| pr64 | gap | yes | yes | yes | yes | yes | — |
| pr64 | networkit | yes | yes | yes | yes | — | — |
| sssp | gap | yes | yes | yes | yes | yes | — |
| sssp | gap-f64 | yes | yes | yes | yes | — | — |
| sssp | pasgal | yes | yes | yes | yes | yes | — |
| tc | gap | yes | yes | yes | yes | — | — |
| tc | gbbs | yes | yes | yes | yes | — | — |
| tc | networkit | yes | yes | yes | yes | — | — |

| kernel | lj | orkut | roadnet-ca | web-google | twitter-sym | friendster |
|---|---|---|---|---|---|---|
| cc | 1 components | 1 components | 2638 components | 2746 components | 1 components | 1 components |
| kcore | max core 360 | max core 253 | max core 3 | max core 44 | max core 2488 | max core 304 |
| bfs | 3997962 reached, max distance 14 | 3072441 reached, max distance 8 | 1957027 reached, max distance 794 | 855802 reached, max distance 15 | 41652230 reached, max distance 15 | 65608366 reached, max distance 22 |
| sssp | 3997962 reached, max distance 1332 | 3072441 reached, max distance 538 | 1957027 reached, max distance 88006 | 855802 reached, max distance 1593 | 41652230 reached, max distance 1803 | — |
| tc | 177820130 triangles | 627584181 triangles | 120676 triangles | 13391903 triangles | — | — |

"—": not run there (NetworKit and igraph only on the four small graphs; triangles not on
the big two; GAP's f64 SSSP needs a 16-byte-per-entry file, not made for twitter-sym;
Friendster: components, k-core and BFS only). What the outputs agree on (Everygraph's
values; every tool above matches them):

Two outputs differ by construction; they are not checked and their cases are
`unavailable` in the spec:

- **GBBS PageRank** returns its zeroed work buffer when it stops at the iteration cap:
  `benchmarks/PageRank/PageRank.h` zeroes `p_curr` while it computes the L1 change, then
  swaps, then returns `p_next`. The first check found it: L1 distance to ours 1, the
  whole mass (`results/validation-first-lj.json`, lj, with the tool builds of that first
  run). With a tolerance instead, it would not
  run 20 iterations.
- **igraph PageRank** is PRPACK's exact solve, not 20 iterations.

Found and fixed while writing the checks: PASGAL's SSSP on a graph marked symmetric
also relaxes v → u at the cost of u → v (its pull step), which assumes equal costs both
ways; the costs here are per entry, so the driver gives PASGAL the graph as directed.

## Memory peaks (one run of each case)

From the cgroup of each run (`results/once.jsonl`: exploratory harness rows, 16 threads,
the commands of the spec at this branch; every one of the 193 cases of orkut,
roadnet-ca, web-google and Friendster ran `ok`). `anon` is the heap; `memory.peak` adds
the page cache charged to the run, which depends on which run read a file first, so
`anon` is the column to compare. Measured, not assumed: GBBS's `anon` holds the graph
even though it maps the file (orkut 1.07 GiB, Friendster 16.2 GiB). The Python tools
include the interpreter and the arrays they build the graph from. The CSR itself is
0.90 GiB (orkut) and 13.9 GiB (Friendster).

| case | status | anon peak GiB | memory.peak GiB | file peak GiB |
|---|---|---|---|---|
| pagerank-eg/graph=orkut/t16 | ok | 1.02 | 1.03 | 0.00 |
| pagerank-gap/graph=orkut/t16 | ok | 1.88 | 1.89 | 0.00 |
| pagerank-networkit/graph=orkut/t16 | ok | 6.26 | 6.28 | 0.00 |
| cc-eg/graph=orkut/t16 | ok | 0.96 | 0.96 | 0.00 |
| cc-gap/graph=orkut/t16 | ok | 1.84 | 1.87 | 0.00 |
| cc-gbbs/graph=orkut/t16 | ok | 1.07 | 1.09 | 0.00 |
| cc-connectit/graph=orkut/t16 | ok | 1.06 | 1.07 | 0.00 |
| cc-networkit/graph=orkut/t16 | ok | 6.26 | 6.28 | 0.00 |
| kcore-eg/graph=orkut/t16 | ok | 1.04 | 1.04 | 0.00 |
| kcore-gbbs/graph=orkut/t16 | ok | 1.16 | 1.16 | 0.00 |
| kcore-pkc/graph=orkut/t16 | ok | 2.40 | 2.42 | 0.00 |
| kcore-networkit/graph=orkut/t16 | ok | 6.26 | 6.28 | 0.00 |
| bfs-eg/graph=orkut/t16 | ok | 0.98 | 1.85 | 0.87 |
| bfs-gap/graph=orkut/t16 | ok | 1.85 | 1.92 | 0.06 |
| bfs-gbbs/graph=orkut/t16 | ok | 1.08 | 1.71 | 0.63 |
| bfs-pasgal/graph=orkut/t16 | ok | 2.19 | 2.22 | 0.00 |
| sssp-eg/graph=orkut/t16 | ok | 2.74 | 2.75 | 0.00 |
| sssp-gap/graph=orkut/t16 | ok | 2.80 | 4.59 | 1.77 |
| sssp-gap-f64/graph=orkut/t16 | ok | 4.56 | 8.10 | 3.52 |
| sssp-pasgal/graph=orkut/t16 | ok | 2.78 | 3.89 | 1.11 |
| tc-gap/graph=orkut/t16 | ok | 2.90 | 3.83 | 0.92 |
| tc-gbbs/graph=orkut/t16 | ok | 1.60 | 1.85 | 0.23 |
| tc-networkit/graph=orkut/t16 | ok | 8.46 | 8.51 | 0.03 |
| leiden-lab/graph=orkut/t16 | ok | 2.23 | 2.42 | 0.00 |
| leiden-lab-det/graph=orkut/t16 | ok | 2.40 | 2.65 | 0.00 |
| leiden-gve/graph=orkut/type=f32/t16 | ok | 8.50 | 10.17 | 1.65 |
| leiden-gve/graph=orkut/type=f64/t16 | ok | 10.93 | 10.95 | 0.00 |
| leiden-gve/graph=roadnet-ca/type=f32/t1 | ok | 0.45 | 0.49 | 0.04 |
| leiden-gve/graph=roadnet-ca/type=f32/t16 | ok | 0.67 | 0.67 | 0.00 |
| leiden-gve/graph=roadnet-ca/type=f64/t1 | ok | 0.50 | 0.50 | 0.00 |
| leiden-gve/graph=roadnet-ca/type=f64/t16 | ok | 0.72 | 0.72 | 0.00 |
| leiden-gve/graph=web-google/type=f32/t1 | ok | 0.40 | 0.46 | 0.06 |
| leiden-gve/graph=web-google/type=f32/t16 | ok | 0.50 | 0.51 | 0.00 |
| leiden-gve/graph=web-google/type=f64/t1 | ok | 0.49 | 0.50 | 0.00 |
| leiden-gve/graph=web-google/type=f64/t16 | ok | 0.59 | 0.59 | 0.00 |
| leiden-networkit/graph=orkut/t16 | ok | 9.07 | 9.12 | 0.02 |
| louvain-networkit/graph=orkut/t16 | ok | 8.50 | 8.52 | 0.00 |
| friendster-eg/kernel=cc/t16 | ok | 15.16 | 22.06 | 7.75 |
| friendster-eg/kernel=kcore/t16 | ok | 16.11 | 22.97 | 10.17 |
| friendster-eg/kernel=bfs/t16 | ok | 15.48 | 28.70 | 13.17 |
| friendster-gap/kernel=cc/t16 | ok | 28.89 | 31.29 | 8.11 |
| friendster-gap/kernel=bfs64/t16 | ok | 28.85 | 40.00 | 19.73 |
| friendster-gbbs/kernel=cc/t16 | ok | 16.23 | 37.42 | 21.38 |
| friendster-gbbs/kernel=connectit/t16 | ok | 15.97 | 22.67 | 6.95 |
| friendster-gbbs/kernel=kcore/t16 | ok | 17.04 | 22.61 | 6.41 |
| friendster-gbbs/kernel=bfs64/t16 | ok | 16.20 | 21.71 | 6.82 |
| friendster-ucr/kernel=pkc/kcore/t16 | ok | 37.31 | 37.41 | 7.61 |
| friendster-ucr/kernel=pasgal/bfs/t16 | ok | 34.21 | 40.00 | 16.41 |
| friendster-cost | ok | 14.49 | 27.54 | 13.02 |

## Found without timing

- **Everygraph's BFS with parents** costs about 4.3 times its BFS without them on orkut
  (16 threads, from the hub: 0.112–0.121 s against 0.026–0.028 s best of 5, over 4
  interleaved rounds of one binary; `results/ab-bfs-parents-orkut.txt`, exploratory).
  The cause is in `crates/everygraph-kernels/src/traversal.rs`: the deterministic parent
  (the smallest node one hop closer, `docs/api-sketch.md` § 2.12) is found by a second
  pass over every reached node's whole list, with an atomic minimum per hit, after the
  direction-optimizing traversal, which reads far fewer entries. GAP and GBBS keep the
  first parent their traversal writes. The spec times BFS with parents, as the GAP rules
  ask, so this cost is in every BFS row of the official run. It is a candidate for the
  M3 performance pass.

## What each tool runs

| tool | version | kernels | statistic | load |
|---|---|---|---|---|
| Everygraph | this branch, `bench/references/eg-driver` (the product kernels unchanged) | `pagerank` (pull, Jacobi, f64, strength included), `components::wcc` (hybrid sampling), `kcore` (Liu et al.), `traversal::bfs` (direction-optimizing, parents), `sssp` (Δ-stepping, data-rule width, f64) | median of 5; searches: mean of 64 | arrays read into the heap, timed apart |
| GAP | 941462c | `pr_spmv` (Jacobi) with `ScoreT = double`, `cc` (Afforest), `bfs` (direction-optimizing, parents), `sssp` (Δ-stepping, Δ = 2; int32, and `WeightT = double`), `tc` | its trial timer: median of 5; searches: mean of 64 | `.sg` read and built, timed apart |
| GBBS | b1a90f5 (2023-05-18, the last commit with makefiles), `gbbs_driver.cc` | `WorkEfficientSDB14` connectivity, `JulienneDBS17` k-core, `NonDeterministicBFS` (parents), `ShunTangwongsan15` triangles | median of 5 rounds; BFS mean of 64 | binary graph memory-mapped, not timed |
| ConnectIt | in the GBBS tree | UF-Rem-CAS, k-out sampling, FindAtomicSplit, SplitAtomicOne (its paper's fastest) | median of 5 | as GBBS |
| Parallel-KCore | b4966c8 (no licence: built here, never committed), `ucr_driver.cc` | `KCore::kcore()` | median of 5 | `.bin` read into the heap |
| PASGAL | f7817ba, `ucr_driver.cc` | BFS (distances), rho-stepping SSSP (uint32) | mean of 64 | `.bin` read into the heap |
| COST line | frankmcsherry/COST d5bef75, `bench/references/cost-line` | union-find by rank, PageRank (f64), one thread, edges in CSR order | median of 5 | arrays read into the heap |
| NetworKit | 11.3 | `PageRank` (f64, 20 iterations), `ParallelConnectedComponents`, `CoreDecomposition`, `BFS` (sequential, distances), `TriangleEdgeScore` (+ `indexEdges`), `ParallelLeiden`, `PLM` | median of 5 calls; BFS mean of 64; Leiden one | graph built from the CSR arrays, timed apart |
| python-igraph | 1.0.0 | `connected_components`, `coreness`, `bfs` (Python lists), `community_leiden` (2 iterations) | median of 5 calls; BFS mean of 64; Leiden one | text edge list read, timed apart |
| GVE-Leiden | f741a40 | `leidenStaticOmp`, f32 and f64 | the binary's own time, one run | MatrixMarket read by the binary |
| lab Leiden | `lab/b2-leiden`, this branch | Leiden, asynchronous and `--det` | `time_s`, one run | CSR memory-mapped |

## Not done

- The official runs, so no ratio and no "where we are slower" section yet: that comes
  from the official orkut window, with intervals (#194, #195).
- A `-march=native` variant run next to the generic one, and an unpinned one: the
  native flavour is built (`build.sh native`, `{work}/eg-driver-native`), not in the spec.
- Vertex orders other than the input order (protocol rule 1: degree, reverse degree).
- Large road and web graphs (europe-osm or DIMACS USA, uk-2005): not in the registry;
  roadnet-ca and web-google are small.
- GBBS SSSP; scipy, DuckDB and the F2 query engines in this spec (they are in the
  `baselines` and `engines` specs); COST's Hilbert order; PageRank and SSSP on
  Friendster; NetworKit and igraph on twitter-sym and Friendster.
- The regression gate of #55 (waits for #194).

## Files

- `interleave.py`: the exploratory interleaved rounds (the raw rows in `results/`).
- `tables.py`: the tables above, from `results/validation-*.json` and `results/once.jsonl`.
