# m3-12 — reference runs: Everygraph against the best-known kernel codes (#55, #265)

Question: on this machine, how do Everygraph's production kernels compare with the best
tools — GAP, GBBS, ConnectIt, Parallel-KCore, PASGAL, the COST line, NetworKit,
python-igraph, GVE-Leiden — on the same graphs, inputs and thread counts?

**There are no speed ratios between tools here.** Such a ratio is a result only from an
official run on an idle machine, with intervals (CLAUDE.md § 3; #265): exploratory
rounds on this shared machine changed the sign of one between rounds (#265 quotes it).

What this directory holds, all independent of the machine's load:

- **validated outputs**: every tool's result checked against Everygraph's (below);
- **memory peaks** of one run of each orkut, roadnet-ca, web-google and Friendster case;
- two same-code A/B ratios (below), and the method, in `bench/specs/references.toml`.

## Official commands

The orkut window (the review's experiment F4: every reference tool, validated, 6
repetitions; the report gives the intervals from the same rows, #194). The preparation
(builds, conversions, downloads) runs before the window; the timed commands skip it:

```sh
everygraph-bench run references --prepare-only --only graph=orkut/ --only validate/on=orkut --data /home/tux/graphlab/data
everygraph-bench run references --official --skip-prepare --only validate/on=orkut --data /home/tux/graphlab/data
everygraph-bench run references --official --skip-prepare --only graph=orkut/ --repetitions 6 --data /home/tux/graphlab/data
```

The whole spec: the same three commands without `--only` and `--repetitions`.

Duration, an estimate from one exploratory pass (`results/once.jsonl`, a loaded machine):
one repetition of the orkut cases took 32 min of wall time, so the window takes about
3 h at 6 repetitions, plus about 10 min of validation. The whole spec takes about 8 h at
3 repetitions (an extrapolation: twitter-sym and lj were not timed in that pass).

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

- **Same work.**
  - PageRank: f64, damping 0.85, 20 iterations, tolerance 0. GAP's damping and 1/n
    constants stay f32 (only its `ScoreT` is double), which is why its L1 distance to
    ours is about 4e-8 and the others' about 1e-16.
  - BFS and SSSP: from the same 64 seeded non-zero-degree sources, GAP's own
    (`gap_sources.cc` prints SourcePicker's list). BFS returns parents (Everygraph, GAP,
    GBBS, igraph) or distances (PASGAL, NetworKit).
  - SSSP costs: every tool gets the cost of the undirected pair, an integer from 1 to
    255 drawn from {min(u, v), max(u, v)} (`pair_costs` in `run.py`). A graph with equal
    costs both ways stays symmetric, so PASGAL keeps its pull step. Precision: f64
    (Everygraph, GAP with `WeightT = double`) and integers (GAP int32, PASGAL uint32);
    integer sums are exact in both, so the distances are equal. Everygraph computes its
    bucket width once, before the timed searches; GAP's Δ = 2 came from an untimed sweep.
    On twitter-sym there is no f64 GAP: its 16-byte records would need 38 GB.
  - Leiden: 2 Leiden iterations for the lab Leiden, NetworKit's `ParallelLeiden` and
    igraph, each tool's defaults otherwise. GVE-Leiden has no iteration count and runs to
    its own convergence. The settings are not equal, so the communities group gives no
    time ratio: each time stands next to its modularity and that modularity's spread
    over the repetitions (`tables.py --times`).
- **Same statistic.** Every process runs a whole-graph kernel 5 times and reports the
  median. It runs a search once per source and reports the mean, and runs Leiden once.
  The harness repeats the process.
- **Same flags.** Every C++ tool and Everygraph without `-march=native` /
  `target-cpu=native` (`bench/references/build.sh generic`; GBBS's makefile flag
  removed), each with its own optimization level (`-O3`; Rust release with fat LTO).
- **Placement.** Every process runs under `bench/references/pin.sh` on the 16 physical
  cores (`taskset -c 0-15`). OpenMP threads (GAP, NetworKit, GVE) are bound one per
  core; Rayon and Parlay threads (Everygraph, the lab Leiden, GBBS, ConnectIt,
  Parallel-KCore, PASGAL) keep only the mask, because those runtimes have no such
  binding. Each row's `placement` condition says which.
- **Storage stated.** GBBS maps its graph file (`-b`); the lab Leiden maps the CSR;
  every other tool reads the graph into the heap. Load time is apart from kernel time
  where the tool reports it.

## Validated outputs

`bench/runners/run.py validate` runs each tool once at 16 threads, its result written
out, and compares it with Everygraph's:

- the partition into components (labels renumbered by first appearance);
- every core number;
- every distance from the first of the 64 sources (a parent array is turned into depths,
  and every parent must be a neighbour);
- PageRank by L1 distance (at most 1e-6);
- triangle counts between the tools (Everygraph has no triangle kernel);
- Leiden and Louvain: each modularity within 0.10 of the best tool's on the graph. The
  partitions legitimately differ: the working tools lie up to 0.049 below the best
  (GVE-Leiden f64 on orkut), and a broken run falls far below. The validation run's
  checks were made with a floor of 0.05, which every tool passed; the files show the
  floor of 0.10, computed from the same modularities.

The 1-thread rows are not validated; these kernels' outputs do not depend on the thread
count. A `validate` case fails when a check fails or is skipped (`invalid` and `skipped`
must be 0), and `tables.py` drops every row whose tool did not pass the last
validation of its graph and kernel (`test_tables.py`). Results:
`results/validation-<graph>.json` (one line each, as `run.py validate` prints it), from
the spec's `validate` cases (rows: `results/validate.jsonl`).

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
| leiden | gve-f32 | yes | yes | yes | yes | — | — |
| leiden | gve-f64 | yes | yes | yes | yes | — | — |
| leiden | igraph | yes | yes | yes | yes | — | — |
| leiden | lab | yes | yes | yes | yes | — | — |
| leiden | lab-det | yes | yes | yes | yes | — | — |
| leiden | networkit-leiden | yes | yes | yes | yes | — | — |
| leiden | networkit-plm | yes | yes | yes | yes | — | — |
| pr64 | cost | yes | yes | yes | yes | yes | — |
| pr64 | gap | yes | yes | yes | yes | yes | — |
| pr64 | gbbs | yes | yes | yes | yes | yes | — |
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
| sssp | 3997962 reached, max distance 1156 | 3072441 reached, max distance 600 | 1957027 reached, max distance 91320 | 855802 reached, max distance 1556 | 41652230 reached, max distance 1256 | — |
| tc | 177820130 triangles | 627584181 triangles | 120676 triangles | 13391903 triangles | — | — |

"—": not run there. NetworKit and igraph run only on the four small graphs. Triangles
and Leiden do not run on the big two. Friendster runs components, k-core and BFS only.
The second table gives Everygraph's values, which every tool above matches.

GBBS's PageRank is validated after a one-line patch (`build.sh`): it zeroes `p_curr`
while it sums the change and then swaps, so at the iteration cap its result is in
`p_curr`, not in the `p_next` it returned. Unpatched, its output was all zeros: L1
distance to ours 1 (`results/first-check-lj.json`, with the builds of that first check).
The patch does not change the timed iterations. igraph's PageRank is not run: it is PRPACK's exact solve, not 20
iterations.

## Memory peaks (one run of each case)

From the cgroup of each run (`results/once.jsonl`: exploratory harness rows of every
case of orkut, roadnet-ca, web-google and Friendster, all `ok`; the table shows orkut at
16 threads and Friendster). The SSSP, COST-line, GBBS PageRank and Leiden rows come from
a second pass at revision 83167c3, after their commands changed; the others are from
revision a6cd2c9, whose commands for them are the same. `anon` is the heap. `memory.peak` adds
the page cache charged to the run, which depends on which run read a file first, so
`anon` is the column to compare. GBBS's `anon` holds the graph even though it maps the
file. The Python tools include the interpreter and the arrays they build the graph
from. The CSR itself is 0.90 GiB (orkut) and 13.9 GiB (Friendster). Only rows whose tool
passed its validation are shown.

| case | anon peak GiB | memory.peak GiB |
|---|---|---|
| pagerank-eg/graph=orkut/t16 | 1.02 | 1.03 |
| pagerank-gap/graph=orkut/t16 | 1.88 | 1.89 |
| pagerank-networkit/graph=orkut/t16 | 6.26 | 6.28 |
| cc-eg/graph=orkut/t16 | 0.96 | 0.96 |
| cc-gap/graph=orkut/t16 | 1.84 | 1.87 |
| cc-gbbs/graph=orkut/t16 | 1.07 | 1.09 |
| cc-connectit/graph=orkut/t16 | 1.06 | 1.07 |
| cc-networkit/graph=orkut/t16 | 6.26 | 6.28 |
| kcore-eg/graph=orkut/t16 | 1.04 | 1.04 |
| kcore-gbbs/graph=orkut/t16 | 1.16 | 1.16 |
| kcore-pkc/graph=orkut/t16 | 2.40 | 2.42 |
| kcore-networkit/graph=orkut/t16 | 6.26 | 6.28 |
| bfs-eg/graph=orkut/t16 | 0.98 | 1.85 |
| bfs-gap/graph=orkut/t16 | 1.85 | 1.92 |
| bfs-gbbs/graph=orkut/t16 | 1.08 | 1.71 |
| bfs-pasgal/graph=orkut/t16 | 2.19 | 2.22 |
| sssp-eg/graph=orkut/t16 | 2.74 | 2.75 |
| sssp-gap/graph=orkut/t16 | 2.80 | 4.58 |
| sssp-gap-f64/graph=orkut/t16 | 4.55 | 8.09 |
| sssp-pasgal/graph=orkut/t16 | 2.78 | 2.81 |
| tc-gap/graph=orkut/t16 | 2.90 | 3.83 |
| tc-gbbs/graph=orkut/t16 | 1.60 | 1.85 |
| tc-networkit/graph=orkut/t16 | 8.46 | 8.51 |
| leiden-lab/graph=orkut/t16 | 2.88 | 3.00 |
| leiden-lab-det/graph=orkut/t16 | 3.16 | 3.22 |
| leiden-gve/graph=orkut/type=f32/t16 | 8.50 | 10.17 |
| leiden-gve/graph=orkut/type=f64/t16 | 10.93 | 10.95 |
| leiden-networkit/graph=orkut/t16 | 8.99 | 9.09 |
| louvain-networkit/graph=orkut/t16 | 8.50 | 8.52 |
| friendster-eg/kernel=cc/t16 | 15.16 | 22.06 |
| friendster-eg/kernel=kcore/t16 | 16.11 | 22.97 |
| friendster-eg/kernel=bfs/t16 | 15.48 | 28.70 |
| friendster-gap/kernel=cc/t16 | 28.89 | 31.29 |
| friendster-gap/kernel=bfs64/t16 | 28.85 | 40.00 |
| friendster-gbbs/kernel=cc/t16 | 16.23 | 37.42 |
| friendster-gbbs/kernel=connectit/t16 | 15.97 | 22.67 |
| friendster-gbbs/kernel=kcore/t16 | 17.04 | 22.61 |
| friendster-gbbs/kernel=bfs64/t16 | 16.20 | 21.71 |
| friendster-ucr/kernel=pkc/kcore/t16 | 37.31 | 37.41 |
| friendster-ucr/kernel=pasgal/bfs/t16 | 34.21 | 40.00 |
| friendster-cost | 14.48 | 21.19 |
| pagerank-gbbs/graph=orkut/t16 | 1.17 | 1.20 |

## Same-code A/B ratios (exploratory, interleaved)

- **Everygraph's BFS with parents takes about 4.3 times as long as its BFS without
  them** (orkut, 16 threads, from the hub, 4 interleaved rounds of one binary;
  `results/ab-bfs-parents-orkut.txt`). The cause is in
  `crates/everygraph-kernels/src/traversal.rs`. The deterministic parent (the smallest
  node one hop closer, `docs/api-sketch.md` § 2.12) is found by a second pass over every
  reached node's whole list, with an atomic minimum per hit. That pass runs after the
  direction-optimizing traversal, which reads far fewer entries. GAP and GBBS keep the
  first parent their traversal writes. The spec times BFS with parents, as the GAP
  rules ask, so this cost is in every BFS row of the official run. It is a candidate for
  the M3 performance pass.
- **The COST line with safe indexing takes 1.2–1.3 times as long as with COST's own
  unchecked indexing** (orkut, one thread, union-find and PageRank, 3 interleaved
  rounds; `results/ab-cost-indexing-orkut.txt`). The line therefore uses the unchecked
  form, as COST does.

## What each tool runs

| tool | version | kernels | statistic | load |
|---|---|---|---|---|
| Everygraph | this branch, `bench/references/eg-driver` (the product kernels unchanged) | `pagerank` (pull, Jacobi, f64, strength included), `components::wcc` (hybrid sampling), `kcore` (Liu et al.), `traversal::bfs` (direction-optimizing, parents), `sssp` (Δ-stepping, data-rule width, f64) | median of 5; searches: mean of 64 | arrays read into the heap, timed apart |
| GAP | 941462c | `pr_spmv` (Jacobi) with `ScoreT = double`, `cc` (Afforest), `bfs` (direction-optimizing, parents), `sssp` (Δ-stepping, Δ = 2; int32, and `WeightT = double`), `tc` | its trial timer: median of 5; searches: mean of 64 | `.sg` read and built, timed apart |
| GBBS | b1a90f5 (2023-05-18, the last commit with makefiles), `gbbs_driver.cc` | `PageRank` (dense edgeMapReduce, double; the return patched), `WorkEfficientSDB14` connectivity, `JulienneDBS17` k-core, `NonDeterministicBFS` (parents), `ShunTangwongsan15` triangles | median of 5 rounds; BFS mean of 64 | binary graph memory-mapped, not timed |
| ConnectIt | in the GBBS tree | UF-Rem-CAS, k-out sampling, FindAtomicSplit, SplitAtomicOne | median of 5 | as GBBS |
| Parallel-KCore | b4966c8 (no licence: built here, never committed), `ucr_driver.cc` | `KCore::kcore()` | median of 5 | `.bin` read into the heap |
| PASGAL | f7817ba, `ucr_driver.cc` | BFS (distances), rho-stepping SSSP (uint32, symmetric) | mean of 64 | `.bin` read into the heap |
| COST line | frankmcsherry/COST d5bef75 (MIT), `bench/references/cost-line` | union-find by rank, PageRank (f64), one thread, edges in CSR order, unchecked indexing | median of 5 | arrays read into the heap |
| NetworKit | 11.3 | `PageRank` (f64, 20 iterations), `ParallelConnectedComponents`, `CoreDecomposition`, `BFS` (sequential, distances), `TriangleEdgeScore` (+ `indexEdges`), `ParallelLeiden` (2 iterations), `PLM` | median of 5 calls; BFS mean of 64; Leiden one | graph built from the CSR arrays, timed apart |
| python-igraph | 1.0.0 | `connected_components`, `coreness`, `bfs` (parents, Python lists), `community_leiden` (2 iterations) | median of 5 calls; BFS mean of 64; Leiden one | text edge list read, timed apart |
| GVE-Leiden | f741a40 | `leidenStaticOmp`, f32 and f64, its own convergence | the binary's own time, one run | MatrixMarket read by the binary |
| lab Leiden | `lab/b2-leiden`, this branch | Leiden, asynchronous and `--det`, 2 iterations | `time_s`, one run | CSR memory-mapped |

## Not done

- The official runs, so no ratio between tools and no "where we are slower" section
  yet. Both come from the official orkut window, with intervals (#194, #195).
- A `-march=native` variant (`build.sh native` builds it, no case runs it) and an
  unbound variant.
- Vertex orders other than the input order (protocol rule 1: degree, reverse degree).
- Large road and web graphs (europe-osm or DIMACS USA, uk-2005): not in the registry;
  roadnet-ca and web-google are small.
- GBBS SSSP; scipy, DuckDB and the F2 query engines in this spec (they are in the
  `baselines` and `engines` specs); COST's Hilbert order; PageRank and SSSP on
  Friendster; NetworKit and igraph on twitter-sym and Friendster.
- The regression gate of #55 (waits for #194).

## Files

- `tables.py`: the tables above, from `results/validation-*.json` and harness rows
  (`python3 tables.py results results/once.jsonl`; `--times` for official rows);
  `test_tables.py`: the gate that drops unvalidated rows.
- `results/`: the validation files and rows, the one-run pass (`once.jsonl`), the two
  A/B files.
