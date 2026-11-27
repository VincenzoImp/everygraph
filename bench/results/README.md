# Official results

This directory holds the rows of **official** benchmark runs: an idle machine, the
benchmark harness, one result file per run (CLAUDE.md § 3; the protocol is in
[bench/README.md](../README.md)). Each file is named `<spec>/<timestamp>-<revision>.jsonl`.
Every row records the command, the machine, the load, the memory limit and the peak, and
the conditions of the case.

The tables below are a short summary of the newest product results. **They come from
development builds on one machine.** They say where the code stands today. They are not
the numbers of a release, and they change as the performance pass (#240) lands. The
measurements of the research prototypes are in [docs/FINDINGS.md](../../docs/FINDINGS.md).

The machine: AMD Ryzen 9 7950X (16 cores, 32 threads), 61 GB RAM, NVMe, Linux 6.12.
Builds: release, fat LTO, without `target-cpu=native`.

## Everygraph's kernels and reference codes, on orkut

[`references/20261127T023349Z-2510b9c16c0a.jsonl`](references/20261127T023349Z-2510b9c16c0a.jsonl)
(revision 2510b9c1). Before the timed cases, every tool's output was checked against
Everygraph's: 32 checks, 0 invalid
([`references/20261127T022357Z-2510b9c16c0a.jsonl`](references/20261127T022357Z-2510b9c16c0a.jsonl)).

com-Orkut: 3,072,441 nodes, 117,185,083 undirected edges, as uncompressed arrays in
memory (GBBS maps its own binary file). 16 threads on the 16 physical cores. Seconds of
kernel time: the median of 6 processes, each of which reports the median of 5 runs (BFS
and SSSP: the mean over the same 64 sources). NetworKit is called from Python. The method
is in [`bench/specs/references.toml`](../specs/references.toml).

| Kernel | Everygraph | GAP | GBBS | Other | NetworKit |
|---|---|---|---|---|---|
| PageRank, f64, 20 iterations | 0.505 | 0.456 | 0.502 | — | 2.74 |
| Connected components | 0.0181 | 0.0127 | 0.0344 | ConnectIt 0.0190 | 0.251 |
| Core numbers | 0.572 | — | 0.860 | Parallel-KCore 0.455 | 0.584 |
| BFS with parents | 0.0836 | 0.0156 | 0.0192 | PASGAL 0.0247 (distances) | — |
| SSSP, f64 costs | 0.232 | 0.185 | — | — | — |
| SSSP, integer costs | — | 0.128 | — | PASGAL 0.227 | — |

Where Everygraph is slower (ratios of the medians above): GAP is faster on every kernel
it runs, by 1.1× on PageRank, 1.4× on components, 1.25× on SSSP with f64 costs and 5.4×
on BFS; Parallel-KCore is faster by 1.26× on core numbers. Our BFS gives each node the
smallest parent one hop closer, so that the result is the same at any thread count; that
costs a second pass over each reached list. The gaps and their causes are in the
performance pass, [#240](../../plan/issues/m3-22-performance-pass.md). PageRank changed
after this run (PR #339); the next table is after it.

## PageRank on the default storage, orkut

[`edge-map/20261127T062155Z-3b3befb5cf63.jsonl`](edge-map/20261127T062155Z-3b3befb5cf63.jsonl)
(revision 3b3befb5). PageRank, 20 iterations, graph in memory. Seconds of kernel time:
the median of 3 processes, each of which reports the best of 5 runs. Peak memory of the
process, from its cgroup.

| State | Storage | 1 thread | 16 threads | Peak memory |
|---|---|---|---|---|
| f32 | uncompressed arrays | 3.65 | 0.405 | 0.98 GiB |
| f32 | compressed (the default) | 5.12 | 0.369 | 0.55 GiB |
| f64 | uncompressed arrays | 4.39 | 0.462 | 1.04 GiB |
| f64 | compressed (the default) | 5.60 | 0.419 | 0.61 GiB |

## Connected components on four graphs, default storage

[`components-kcore/20261127T064455Z-3b3befb5cf63.jsonl`](components-kcore/20261127T064455Z-3b3befb5cf63.jsonl)
(revision 3b3befb5). The compressed graph read into memory. Seconds of kernel time, as
in the table above.

| Graph | Undirected edges | 1 thread | 16 threads | Peak memory |
|---|---|---|---|---|
| lj | 34,681,189 | 0.124 | 0.0152 | 0.22 GiB |
| orkut | 117,185,083 | 0.109 | 0.0137 | 0.52 GiB |
| twitter-sym | 1,202,513,046 | 2.28 | 0.189 | 5.10 GiB |
| friendster | 1,806,067,135 | 5.22 | 0.476 | 10.65 GiB |

Not in these tables: the product's ingest and community detection, and runs under a
memory budget smaller than the graph. They have no official product run yet.

## Reading a file

```sh
everygraph-bench report bench/results/references/20261127T023349Z-2510b9c16c0a.jsonl
```

prints each case with its statistics. The other spec directories hold earlier runs and
the runs of the research prototypes; [docs/FINDINGS.md](../../docs/FINDINGS.md) says
which file backs each of its numbers.
