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
- [ ] Output validated with the Graphalytics validator on every dataset, and with `everygraph-oracle ldbc` (the specification's match rules) before a time is reported
- [ ] The run rules, as {{#m1-20-ldbc-graphalytics}} recorded them from specification v1.0.5 (§ 2.2.3, § 2.5.3, § 3.2.1): each algorithm runs 3 times per graph; the time limit is 15 min for size class S, 30 min for M, 1 h for L, 2 h for XL, 3 h for 2XL and above (scale = ⌊10 · log₁₀(n + m)⌋ / 10); a run that times out reports the limit as its time; per run: load time T_l (once per graph), processing time T_p, makespan T_m, whether it breached the SLA (a timeout or a failed validation), and EVPS = (n + m) / T_p. "LDBC Benchmark Result" is never used: LDBC's fair-use policy keeps it for audited runs
- [ ] Datasets and disk, planned before the first large run: every dataset of the official set in the registry (`datasets/datasets.toml` has the test graphs, graph500-22 and dota-league today) with its download size; for graph500-30, the `.eg` size and the build's scratch stated first as estimates from bytes per edge, and the build refused by the preflight if the drive cannot hold them
- [ ] **Full disclosure, and a baseline on this machine** (corrected 2026-11-26, `docs/research/sota-review-2026-11.md` S14). LDBC does not audit Graphalytics results; it runs competitions with full disclosure. So each published run carries what a complete result must contain by the specification (v1.0.6-draft § 3.4, as read by the review): target scale, environment, versions, non-default options, T_p, T_m and SLA per run, with Graphalytics' own summaries (arithmetic mean of T_p, throughput from the mean time), which differ from ours. Beside it, a same-machine run of the GraphBLAS (LAGraph) reference platform, `ldbc/ldbc_graphalytics_platforms_graphblas` (Apache-2.0), on the S and M datasets. The LAGraph run is dropped if it does not run here in two days; the disclosure format stays. Pricing (TCO) needs a price list and may need the owner's hands
- [ ] The single-allocation limits at this size, checked: the index of a 447 M-node graph is about 0.84 GB by {{#m2-32-large-sections}}'s arithmetic (15 bits per node; an estimate), under the 1 GiB cap. If a section passes the cap, {{#m2-32-large-sections}} comes first

**Done when.** A table "dataset × algorithm → completed / time / validated" for the full official set on this machine, results files under `bench/results/`, and the published sentence "These are not LDBC Benchmark Results" wherever they appear. Each published run has the full disclosure above, beside the LAGraph baseline where it ran.

**Feeds.** {{#m8-03-public-benchmarks}} (its Graphalytics table is this issue's), the paper ({{#m8-07-paper}}).

**Pointers.** `docs/research/memory-bounded-algorithms.md` (addendum, LDBC section); `docs/research/sota-measurement.md` challenge 4.

**Depends on** {{#m2-11-in-adjacency}} (directed graphs), {{#m2-34-build-columns}} (weights for SSSP), {{#m3-02-planner}}, {{#m3-03-semi-external}} (every scan kernel past RAM), {{#m3-08-triangles}} (LCC), {{#m4-09-label-propagation}} (CDLP), {{#m2-23-block-cache}} (LCC, BFS and SSSP past RAM)

**Informs it, does not block it.** {{#m1-17-chunked-hub-records}} (LCC on hub lists).
