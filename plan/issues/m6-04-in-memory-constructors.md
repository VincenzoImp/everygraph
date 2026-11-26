---
title: Build graphs from in-memory data
milestone: M6 · Python and interop
labels: type:feature, area:python, P1, risk:low, size:S
---
**Why.** Many users start from arrays or a DataFrame, not a file, and small graphs should not require touching the disk.

**Scope**
- [ ] `eg.from_edges(src, dst, …)` for NumPy arrays and Arrow columns; a DataFrame or any Arrow table through `eg.read(table, source=…, target=…)` (`eg.from_dataframe` is folded into it, `docs/api-sketch.md` § 7)
- [ ] `eg.from_scipy(sparse_matrix)`
- [ ] In-memory backing (anonymous) with the same `Graph` API; `g.save("x.eg")` persists it
- [ ] Automatic choice: below a size threshold stay in memory, above it spill to a temp `.eg`; the temporary `.eg` goes to the scratch directory (`temp_dir`, `docs/api-sketch.md` § 2.4) and is removed when the graph object is released, unless `g.save()` keeps it

**Acceptance**
- 10 M edges from NumPy arrays to a usable `Graph` in < 1 s at 16 threads
- Same results as the file-based path (oracle test)

**Depends on** {{#m6-03-arrow-interop}}
