---
title: Arrow interoperability, zero-copy in and out
milestone: M6 · Python and interop
labels: type:feature, area:python, P0, risk:low, size:M, lab-evidence
---
**Why.** Being a good Arrow citizen makes Polars, pandas, pyarrow and DuckDB integrations nearly free. Lab: a hand-written PyCapsule export (~40 lines on arrow-rs `ffi`) was consumed without copying by pyarrow, Polars and pandas; a 4 M-row result became a table in 0.5–4.4 ms.

**Scope**
- [ ] Export: `__arrow_c_array__` / `__arrow_c_stream__` on result tables and property columns; NumPy arrays of numeric columns through Arrow without a copy (`pl.from_arrow(t)["pagerank"].to_numpy()`; no direct `t["pagerank"]` in 0.1, `docs/api-sketch.md` § 7)
- [ ] Import: accept any object implementing the Arrow PyCapsule interface as an edge or node table (Polars/pandas/pyarrow/DuckDB relations) without materializing Python objects
- [ ] **The structure as Arrow CSR** (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S13): a graph, or a view, lends its out-adjacency as two Arrow arrays, `indptr` (`uint64`, n + 1 values) and `indices` (`uint32`, or `uint64` past 2^32 entries), with node and pair property columns beside them, in the layout icebug-format uses (plain `indptr`/`indices` arrays). Zero-copy where the stored layout allows it (raw arrays). On the default compressed storage this is the planner's raw decode tier, materialized: accounted in the budget and refused when it does not fit (8 B per entry for `uint64` indices: Friendster's 3.6 B entries would take about 28.8 GB, arithmetic). Results reached through an external engine on these arrays are outside D14, and the docs say so. This is what {{#m9-03-gql-subset}} bridges to
- [ ] Lifetime safety: exported buffers keep the underlying mapping alive
- [ ] Documented recipes: results → a Polars join with the user's labels (a graph is immutable, so no call attaches a table to it; node sets come back through `subgraph`, `highlight`, `avoid`, `docs/api-sketch.md` § 7); DuckDB query over result tables
- [ ] Tests asserting buffer-address equality (no hidden copies)

**Acceptance**
- Zero-copy tests pass for pyarrow, Polars and pandas (ArrowDtype)
- Round trip Polars → Everygraph → Polars of a 100 M-row edge table without a full Python-side copy

**Depends on** {{#m6-01-python-api}}, {{#m2-06-parquet-reader}}
