---
title: Public, reproducible benchmark report
milestone: M8 · CLI, docs and v0.1 launch
labels: type:docs, area:bench, P0, risk:medium, size:M, lab-evidence
---
**Why.** The adoption pattern shared by DuckDB, Polars and Ruff is one reproducible 10–100× claim with open code. Ours must survive hostile review: the first lab's numbers came from a shared machine and single runs.

**Scope**
- [ ] Report generated from `everygraph-bench` result files ({{#m3-12-perf-tracking}}): ingest, kernels, communities (speed *and* quality), out-of-core under cgroup limits, point queries, map build and transfer sizes
- [ ] Hardware: the lab workstation plus one 16 GB laptop and one cloud instance type, so "laptop" claims are measured on a laptop
- [ ] Comparisons with pinned versions and per-tool notes on what is included; invite maintainers of compared tools to review before publication
- [ ] Default-storage numbers come from FINDINGS § M1-13 with their conditions; raw-array numbers are labelled as such
- [ ] One-command reproduction kit with dataset fetchers
- [ ] A "where we lose" section (e.g. Gephi on styling small graphs, Neo4j on Cypher, GPUs on raw throughput, BV on web-graph compression)
- [ ] Comparison runners left unavailable by {{#m0-03-bench-harness}}: Neo4j + GDS (needs a server-lifecycle runner: download, import, start bound to 127.0.0.1, stop by PID) and DuckPGQ (needs a DuckDB release compatible with the extension, beside the pinned one); each recorded with tool, version, algorithm and options like the others

**Acceptance**
- An outside person reproduces the headline table on their own machine within 20% using only the kit
- No number in the README or launch post is absent from the report

**Depends on** {{#m0-03-bench-harness}}, {{#m3-12-perf-tracking}}
