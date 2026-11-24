---
title: Build graphs from in-memory data
milestone: M6 · Python and interop
labels: type:feature, area:python, P1, risk:low, size:S
---
**Why.** Many users start from arrays or a DataFrame, not a file, and small graphs should not require touching the disk.

**Scope**
- [ ] `eg.from_edges(src, dst, …)` for NumPy arrays and Arrow columns; `eg.from_dataframe(df, source=…, target=…)`
- [ ] `eg.from_scipy(sparse_matrix)`
- [ ] In-memory backing (anonymous) with the same `Graph` API; `g.save("x.eg")` persists it
- [ ] Automatic choice: below a size threshold stay in memory, above it spill to a temp `.eg`

**Acceptance**
- 10 M edges from NumPy arrays to a usable `Graph` in < 1 s at 16 threads
- Same results as the file-based path (oracle test)
