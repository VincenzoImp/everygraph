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

DURATION

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

VALIDATION

Two outputs fail, by construction, and those cases are `unavailable` in the spec:

- **GBBS PageRank** returns its zeroed work buffer when it stops at the iteration cap:
  `benchmarks/PageRank/PageRank.h` zeroes `p_curr` while it computes the L1 change, then
  swaps, then returns `p_next` (L1 distance to ours: 1, the whole mass). With a tolerance
  instead it would not run 20 iterations.
- **igraph PageRank** is PRPACK's exact solve, not 20 iterations (not run).

Found and fixed while writing the checks: PASGAL's SSSP on a graph marked symmetric
also relaxes v → u at the cost of u → v (its pull step), which assumes equal costs both
ways; the costs here are per entry, so the driver gives PASGAL the graph as directed.

## Memory peaks (one run of each case)

From the cgroup of each run (`results/once.jsonl`, exploratory harness rows): `anon` is
the heap; `memory.peak` adds the page cache charged to the run (files read first by it,
mapped files). GBBS and the lab Leiden map the graph, so their `anon` leaves it out. The
Python tools include the interpreter and the arrays they build the graph from.

MEMORY

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
