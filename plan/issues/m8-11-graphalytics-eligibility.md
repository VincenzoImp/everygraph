---
title: LDBC Graphalytics: eligible for the full suite, every algorithm and every dataset
milestone: M8 · CLI, docs and v0.1 launch
labels: type:feature, area:bench, P1, risk:medium, size:L
---
**Why.** The owner's goal (2026-11-26): Everygraph should be able to run every Graphalytics benchmark on one machine — not to win on time against clusters, but to be a valid entry for all six algorithms on all datasets, including graph500-30 (447 M nodes, 17 B edges), which does not fit in 61 GB. {{#m1-20-ldbc-graphalytics}} validates kernels against reference outputs; {{#m8-03-public-benchmarks}} publishes comparisons. This issue makes the full suite run end to end.

**Scope**
- [ ] All six algorithms with Graphalytics semantics: BFS, PageRank (f64, fixed iterations, dangling-node rule as specified), WCC, CDLP (initial label = vertex id, ties to the smallest label — see the CDLP row in `design/DECISIONS.md`), LCC (directed and undirected definitions), SSSP (weighted datasets only)
- [ ] Directed and undirected, weighted and unweighted inputs in Graphalytics' file format (`.v`, `.e`, `.properties`), read by the ingest path ({{#m2-08-csr-build}})
- [ ] A Graphalytics platform driver (the benchmark harness calls a platform through its driver interface): load, run, write output in the expected format; licence and code location decided in the PR (outside the product crates)
- [ ] Every dataset of the official set runs within the benchmark's time limit per job on this machine (61 GB RAM, one NVMe), out of core where needed; the list of datasets, sizes and limits is copied into the PR from the current Graphalytics specification, not from memory
- [ ] LCC on bigger-than-RAM graphs: the known weak case (random access to many neighbour lists); depends on {{#m2-23-block-cache}} and {{#m1-17-chunked-hub-records}}; if it cannot meet the time limit on the largest datasets, record the measured time and the reason
- [ ] Output validated with the Graphalytics validator on every dataset

**Done when.** A table "dataset × algorithm → completed / time / validated" for the full official set on this machine, results files under `bench/results/`, and the published sentence "These are not LDBC Benchmark Results" wherever they appear. An audit by LDBC is a separate, later decision.

**Feeds.** {{#m8-03-public-benchmarks}}, the paper ({{#m8-07-paper}}).

**Pointers.** `docs/research/memory-bounded-algorithms.md` (addendum, LDBC section); `docs/research/sota-measurement.md` challenge 4.
