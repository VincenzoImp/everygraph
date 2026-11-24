---
title: Arrow interoperability, zero-copy in and out
milestone: M6 · Python and interop
labels: type:feature, area:python, P0, risk:low, size:M, lab-evidence
---
**Why.** Being a good Arrow citizen makes Polars, pandas, pyarrow and DuckDB integrations nearly free. Lab: a hand-written PyCapsule export (~40 lines on arrow-rs `ffi`) was consumed without copying by pyarrow, Polars and pandas; a 4 M-row result became a table in 0.5–4.4 ms.

**Scope**
- [ ] Export: `__arrow_c_array__` / `__arrow_c_stream__` on result tables and property columns; NumPy views for plain numeric columns (read-only views into mmap where applicable)
- [ ] Import: accept any object implementing the Arrow PyCapsule interface as an edge or node table (Polars/pandas/pyarrow/DuckDB relations) without materializing Python objects
- [ ] Lifetime safety: exported buffers keep the underlying mapping alive
- [ ] Documented recipes: results → Polars join → back as a node property; DuckDB query over result tables
- [ ] Tests asserting buffer-address equality (no hidden copies)

**Acceptance**
- Zero-copy tests pass for pyarrow, Polars and pandas (ArrowDtype)
- Round trip Polars → Everygraph → Polars of a 100 M-row edge table without a full Python-side copy
