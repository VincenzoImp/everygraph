---
title: LDBC Graphalytics as the large-graph oracle and a benchmark workload
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:bench, P1, risk:low, size:M
---
**Question.** The correctness oracle ({{#m0-05-oracle}}) stops at 10⁵ edges. "The same result for any budget" and the determinism CI ({{#m3-11-determinism-ci}}) have no reference outputs at the scale where out-of-core paths run. LDBC Graphalytics publishes reference outputs for six deterministic algorithms on graphs up to billions of edges, com-friendster included. Do our lab kernels match them? Where they do not, which semantic difference is it?

**What LDBC Graphalytics specifies** (`docs/research/memory-bounded-algorithms.md`, addendum item 17; specification v1.0.5, arXiv 2011.15028 v6):
- BFS depth: exact match.
- PageRank: a fixed number of iterations, **f64 required**, sink mass spread uniformly; match within |r − s| ≤ 10⁻⁴·|r|.
- WCC on the undirected interpretation: match up to relabelling.
- CDLP: synchronous label propagation, initial label = vertex id, ties → smallest label; exact match.
- LCC, the directed definition included: epsilon match.
- Weighted SSSP: epsilon match.
- Datasets: about 350 GB compressed. Tiny test-* graphs per algorithm; Graph500 scales 22–30, the largest graph500-30 with 447 M nodes and 17 B edges; com-friendster and other real graphs.
- Licences: the driver is Apache-2.0. The datasets' licence is not stated on the page (not verified); the real graphs come from SNAP, KONECT and others under their own terms.
- Reporting: "LDBC Benchmark Result" is reserved for audited results. Unaudited numbers must not use the name and say "These are not LDBC Benchmark Results" and how they differ.

**Method.** Downloads go through the dataset registry ({{#m0-04-datasets}}); `raw/` is read-only. Every run that loads a graph runs under `systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0` unless a larger limit is stated.
- [ ] Cheapest step: the test-* and example graphs plus graph500-22 (64 M edges). Validate the lab PageRank (f64), WCC and BFS against the reference outputs with LDBC's match rules. Then CDLP (with the semantics now decided for {{#m4-09-label-propagation}}), LCC and SSSP where a lab kernel exists. Record each mismatch as a semantic difference, with its cause
- [ ] Our PageRank formula S1 (`docs/spec/graph-semantics.md` § 5.1) with uniform teleport against LDBC's in f64; confirm that the f32 default does not qualify ({{#m3-04-pagerank}})
- [ ] com-friendster: the same checks in RAM and under a budget below the graph's size. These are the first large-graph references for "same result for any budget"
- [ ] An `ldbc` test profile proposed for {{#m3-11-determinism-ci}} (which graphs, which algorithms, run time), and each dataset's licence terms recorded in the dataset registry (`datasets/datasets.toml`) before any file is redistributed or published
- [ ] The run rules recorded for {{#m8-03-public-benchmarks}}: 3 repetitions, timeouts by scale class, reported processing time, makespan and EVPS, the disclaimer

**Done when.** A table algorithm × graph × {match, mismatch and its cause} in FINDINGS, the conformance profile proposed, and the reporting rule copied into {{#m8-03-public-benchmarks}}.

**Feeds.** {{#m3-11-determinism-ci}}, {{#m8-03-public-benchmarks}}, {{#m3-04-pagerank}}, {{#m3-05-components}}, {{#m3-06-traversal}}, {{#m3-08-triangles}}, {{#m4-09-label-propagation}}, {{#m1-14-relabel-at-ingest}} (relabelled ids must keep CDLP and min-label semantics on original ids).

**Pointers.** `docs/research/memory-bounded-algorithms.md` addendum items 17 and 18, § F 1, § G; https://ldbcouncil.org/benchmarks/graphalytics/.

**Depends on** {{#m0-04-datasets}}
