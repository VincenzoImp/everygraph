# m3-12 — reference runs: Everygraph against the best-known kernel codes (#55)

Question: on this machine, how do Everygraph's production kernels compare with the best
tools — the GAP Benchmark Suite, GBBS, NetworKit, python-igraph and GVE-Leiden — on the
same graphs, the same inputs and the same thread counts?

**Every time here is exploratory.** The machine was shared with about nine other agents
(1-minute load average 5–20 during the runs, recorded per row as `load_start`). The
quotable result is the **ratio** of each tool's time to Everygraph's time from the same
round: in each round every comparison set (kernel, graph, threads) and every tool inside
it ran in a random order, so background load hits all tools of a set alike. Absolute
seconds are listed only to show the scale; they are not claims. Memory peaks and the
results themselves (hashes, modularity, triangle counts) do not depend on load.

The official run regenerates every table on an idle machine:

```sh
everygraph-bench run references --official --data /home/tux/graphlab/data
```

## Setup

- Spec: `bench/specs/references.toml`. The rows here come from its own commands, run by
  `interleave.py` (rounds, random order, each command under
  `systemd-run --user --scope -p MemoryMax=<limit> -p MemorySwapMax=0` and
  `everygraph-bench measure`, as the harness runs it). `summarize.py` makes the tables.
- Graphs: the registry's CSR (`csr/<graph>`: symmetric, sorted, no self-loops, input
  order), converted once to each tool's format by `bench/runners/run.py convert`
  (same node ids, same order). No graph has isolated vertices (counted from
  `offsets.u64`), so every tool sees the packed graph:

  | graph | n | undirected edges | isolated |
  |---|---|---|---|
  | lj | 3,997,962 | 34,681,189 | 0 |
  | orkut | 3,072,441 | 117,185,083 | 0 |
  | twitter-sym | 41,652,230 | 1,202,513,046 | 0 |
  | friendster | 65,608,366 | 1,806,067,135 | 0 |

- Threads: 1 and 16 (the physical cores; SMT not used); twitter-sym at 16 only;
  Friendster: components, BFS and k-core at 16 only, under a 40 GiB limit. igraph is
  sequential (1 thread); NetworKit's BFS is sequential (1 thread).
- Memory limit: 24 GiB per process (40 GiB on Friendster), no swap. Warm page cache.
- Builds: Everygraph release (fat LTO, no `target-cpu=native`); GAP its Makefile
  (`-O3 -fopenmp`); GBBS its makefiles (`-O3 -march=native`, homegrown scheduler,
  `GBBSLONG`); GVE-Leiden `-O3 -fopenmp -DTYPE=float`.
- Rounds: `results/rounds.jsonl` — round 0 (all tools, interrupted after the first
  sets to save time), rounds 1–3 (Everygraph and the C++ tools, every set), round 10
  (NetworKit and igraph with Everygraph and the lab Leiden in the same sets);
  `results/friendster.jsonl`, rounds 0–2. Summary: `results/summary.json`.

## What each tool runs and times

| tool | version | kernels | kernel time | load |
|---|---|---|---|---|
| Everygraph | this branch's revision, `bench/references/eg-driver` | `pagerank` (pull, f64, strength included), `components::wcc` (hybrid sampling), `kcore` (Liu et al. SIGMOD 2025), `traversal::bfs` (direction-optimizing, distances only), `sssp` (Δ-stepping, width of the data rule, f64 costs) | best of 5 in one process; searches: mean over the 64 sources | arrays read into the heap, timed apart |
| GAP | 941462c (2026-11-26) | `pr_spmv` (Jacobi) and `pr` (Gauss-Seidel) with `ScoreT = double`, `cc` (Afforest), `bfs` (direction-optimizing, parents), `sssp` (Δ-stepping, Δ = 2, int32 costs), `tc` | its trial timer: best of 5 (3 for `tc`); searches: mean of 64 trials | `.sg`/`.wsg` read and built, timed apart (`Read Time` + `Build Time`) |
| GBBS | b1a90f5 (2023-05-18, the last commit with makefiles) | `PageRank` (dense edgeMapReduce, double), `WorkEfficientSDB14` connectivity, `JulienneDBS17` k-core, `NonDeterministicBFS` (parents), `ShunTangwongsan15` triangles | its "Running Time": best of 5 rounds (3 for triangles); BFS: mean over the 64 sources (`bench/references/gbbs_bfs_sources.cc`) | the binary graph memory-mapped (`-b`), not timed |
| NetworKit | 11.3 | `PageRank` (f64, `maxIterations` 20, tol 1e-30), `ParallelConnectedComponents`, `CoreDecomposition`, `BFS` (sequential), `TriangleEdgeScore` (+ `indexEdges`), `ParallelLeiden`, `PLM` | one call, in Python | graph built from the CSR arrays, timed apart |
| python-igraph | 1.0.0 | `pagerank` (PRPACK: an exact solve, not 20 iterations), `connected_components`, `coreness`, `bfs` (returns Python lists), `community_leiden` (modularity, 2 iterations) | one call, in Python | text edge list read, timed apart |
| GVE-Leiden | f741a40 (2025-04-08) | `leidenStaticOmp`, f32 | the binary's own time | MatrixMarket read by the binary, not reported |
| lab Leiden | `lab/b2-leiden`, this revision | Leiden, asynchronous (default) and `--det` | `time_s`, the kernel alone | graph memory-mapped |

PageRank is f64, damping 0.85, 20 iterations, tolerance 0 for Everygraph, GAP and GBBS;
NetworKit stops at 20 iterations too (its tolerance 1e-30 is never reached). Triangles
are exact counts on the undirected graphs (lj: 177,820,130, as SNAP). The 64 BFS and SSSP
sources are GAP's own (SourcePicker, seeded, non-zero degree; `gap_sources.cc` prints
them), the same for every tool.

The memory columns are the cgroup's peaks for the whole process: `anon` is heap,
`memory.peak` adds file pages charged to the run (page cache it read first, and mapped
files). GBBS and the lab Leiden map the graph file, so their `anon` peak leaves the graph
out; its size is the CSR's (lj 0.29 GiB, orkut 0.90 GiB, twitter-sym 9.3 GiB,
Friendster 13.9 GiB). Python tools include the interpreter and the arrays they build
from.

TABLES

## Where we are slower, and the likely cause

GAPS

## Not done

- Vertex orders other than the input order (protocol rule 1: degree and reverse degree).
- At least 6 rounds per case (rule 8): 3–4 here for the C++ tools and Everygraph, 1–2 for
  NetworKit and igraph, whose ratios are large.
- COST (rule 6): the 1-thread lines are here; the thread count at which each parallel tool
  beats Everygraph's 1-thread time was not searched.
- Output validation with Graphalytics' match rules (rule 7). Checked instead: Everygraph's
  hashes equal `edge_map_bench`'s for `pr64`, `cc` and `kcore` on lj; the triangle count
  equals SNAP's; component counts and max cores agree between tools where printed.
- ConnectIt and Liu et al.'s k-core code (issue scope), GBBS SSSP, scipy and DuckDB
  (in `bench/specs/baselines.toml`), Friendster PageRank and SSSP.
- The regression gate (#55's other part) waits for m3-18.
