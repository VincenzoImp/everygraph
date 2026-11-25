---
title: Python API v0
milestone: M6 · Python and interop
labels: type:feature, area:python, P0, risk:medium, size:L, lab-evidence
---
**Why.** Python is the main adoption surface. The lab fixed its shape: results must be bulk and columnar (a per-node call costs 30–60 ns, a Python list copy 323 ns), graphs must be immutable and file-backed, and kernels must release the GIL.

**Scope**
- [ ] `eg.read(path, …)` / `eg.open("graph.eg")` → `Graph`; `Graph.view(filter)` → `View`
- [ ] Algorithms as methods returning Arrow-backed tables: `pagerank()`, `components()`, `communities()`, `kcore()`, `triangles()`, `bfs()`, …
- [ ] Point queries with batch variants: `neighbors(ids)`, `degree(ids)`, `khop(id, k, limit=…, timeout=…)`, `path(a, b)` ({{#m7-01-point-queries}})
- [ ] `explain()` / preflight on every long operation; progress callbacks; cancellation on `KeyboardInterrupt`
- [ ] Options: `threads`, `memory`, `deterministic=True`, `seed`
- [ ] External ids in, external ids out by default; dense ids available for performance
- [ ] Typed exceptions (`InsufficientBudget`, `InvalidInput`, …)
- [ ] Type stubs, docstrings with examples, a consistent naming scheme from {{#m0-11-api-sketch}}
- [ ] PyO3 with abi3-py310, `py.detach` around every kernel

**Acceptance**
- The ten user stories from {{#m0-11-api-sketch}} run as written
- PageRank from Python within 3% of the native binary (lab: indistinguishable within noise)
- No API returns a Python list or dict proportional to n or m

**Pointers.** `lab/f2-python/rust/glabpy`.

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The approved shape of the API and of everything it prints is `design/preview/api.md`; how it renders in a notebook is `design/preview/style/``notebook.html`. Issue {{#m0-11-api-sketch}} turns it into the specification this issue implements.
