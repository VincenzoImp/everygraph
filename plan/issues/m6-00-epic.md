---
title: M6 epic: Python and interop
milestone: M6 · Python and interop
labels: type:epic, area:python, P0
---
Tracking issue for milestone M6.

**Goal.** `pip install everygraph` works on Linux, macOS and Windows; results flow into Polars, pandas, pyarrow and DuckDB without copies; data moves in and out of the tools people already use.

**Lab baseline.** PyO3 abi3 module: zero-copy verified by buffer address; Python reaches native kernel speed with the GIL released; one wheel of about 525 KB (estimate, no result file) for CPython 3.10–3.14; per-call overhead 28–40 ns. NetworkX backend prototype (~200 lines): dispatch and fallback work; 0.92–2.0× on an existing `nx.Graph` including conversion; with native loading, kernel time only, PageRank 15.6–70× and components 24–53× (lj, 3 threads, shared machine; DESIGN D20).

**Note on sequencing.** A minimal binding ({{#m6-01-python-api}}, first slice) is needed from M2 onward because the correctness oracle ({{#m0-05-oracle}}) drives the engine from Python. This milestone is where the Python *product* is finished.

**Exit criteria**
- [ ] {{#m6-01-python-api}}, {{#m6-02-wheels}}, {{#m6-03-arrow-interop}}, {{#m6-04-in-memory-constructors}}
- [ ] {{#m6-05-import-export}}, {{#m6-10-communities-python}}
- The NetworkX backend ({{#m6-06-networkx-backend}}) was moved to M9 by an approved design choice
- The subgraph bridge and the tutorials moved to M7 ({{#m7-09-subgraph-bridge}}, {{#m7-10-tutorials}}): they need the point queries, shortest paths, views and temporal views of M7
- [ ] {{#m6-09-api-gaps}} — names for the API gaps the literature review found, before the 0.1 freeze
- [ ] Map in the notebook: {{#m5-10-jupyter-widget}} (needs the viewer from M5)
- [ ] Milestone close (CLAUDE.md § 6, § 10): the dogfood pass, the cleanup and test audit, the official measurement window, and the report `docs/reports/M6.md`. Its issues are filed when the last P0 of the milestone starts
