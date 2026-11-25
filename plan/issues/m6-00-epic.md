---
title: M6 epic: Python and interop
milestone: M6 · Python and interop
labels: type:epic, area:python, P0
---
Tracking issue for milestone M6.

**Goal.** `pip install everygraph` works on Linux, macOS and Windows; results flow into Polars, pandas, pyarrow and DuckDB without copies; data moves in and out of the tools people already use.

**Lab baseline.** PyO3 abi3 module: zero-copy verified by buffer address; Python reaches native kernel speed with the GIL released; one wheel of about 525 KB (estimate, no result file) for CPython 3.10–3.14; per-call overhead 28–40 ns. NetworkX backend prototype (~200 lines): dispatch and fallback work; 1–3× on an existing `nx.Graph`, 100×+ with native loading.

**Note on sequencing.** A minimal binding ({{#m6-01-python-api}}, first slice) is needed from M2 onward because the correctness oracle ({{#m0-05-oracle}}) drives the engine from Python. This milestone is where the Python *product* is finished.

**Exit criteria**
- [ ] {{#m6-01-python-api}}, {{#m6-02-wheels}}, {{#m6-03-arrow-interop}}, {{#m6-04-in-memory-constructors}}
- [ ] {{#m6-05-import-export}}, {{#m6-07-subgraph-bridge}}
- The NetworkX backend ({{#m6-06-networkx-backend}}) was moved to M9 by an approved design choice
- [ ] {{#m6-08-tutorials}}
- [ ] Map in the notebook: {{#m5-10-jupyter-widget}} (needs the viewer from M5)
