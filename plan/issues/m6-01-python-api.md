---
title: Python API v0
milestone: M6 · Python and interop
labels: type:feature, area:python, P0, risk:medium, size:L, lab-evidence
---
**Why.** Python is the main adoption surface. The lab fixed its shape: results must be bulk and columnar (lj, 3 threads, shared machine: an empty call 28 ns, `degree` of one node 34 ns, a zero-copy neighbour view 62 ns, a copy to a Python list 323 ns; `lab/f2-python/results/goalB_lj_dev.json`), graphs must be immutable and file-backed, and kernels must release the GIL.

**Scope**
- [ ] `eg.read(path, …)` / `eg.open("graph.eg")` → `Graph`; `g.filter(…)`, `g.window(…)`, `g.subgraph(…)` → `View` (the approved names, `docs/api-sketch.md` § 3, § 7)
- [ ] Algorithms as methods returning Arrow-backed tables: `pagerank()`, `components()`, `communities()`, `kcore()`, `triangles()`, `bfs()`, …
- [ ] Point queries with batch variants: `neighbors(ids)`, `degrees(ids)`, `khop(id, k, limit=…, timeout=…)`, `path(a, b)` ({{#m7-01-point-queries}})
- [ ] `explain()` / preflight on every long operation; progress callbacks; cancellation on `KeyboardInterrupt`
- [ ] Options: `threads`, `memory`, `deterministic=True`, `seed`
- [ ] The budget behind `memory=` is `Budget::new(limit).with_pages(everygraph_io::PAGES)`: the engine's large buffers are mapped and given back when freed, which the interpreter's allocator cannot do for them (`design/DECISIONS.md`, "Large buffers get pages of their own"); a test checks that the module's accountant returns `PAGES` from `pages()`
- [ ] External ids in, external ids out by default; dense ids available for performance
- [ ] Typed exceptions (`InsufficientBudget`, `InvalidInput`, …)
- [ ] Type stubs, docstrings with examples, a consistent naming scheme from {{#m0-11-api-sketch}}
- [ ] PyO3 with abi3-py310, `py.detach` around every kernel
- [ ] No signal handler is installed: long calls poll Python's signal check so `Ctrl-C` arrives as a plain `KeyboardInterrupt` after a clean stop (`docs/ux-contract.md` C-6); a result cut short is reported with `warnings.warn(…, eg.TruncatedWarning)` (M-12); every event validates against the event schema of § 4.6

**Acceptance**
- The steps of the ten user stories from {{#m0-11-api-sketch}} that use only this issue's API run as written; the whole stories are the acceptance of {{#m6-08-tutorials}}
- PageRank from Python within 3% of the native binary (lab: indistinguishable within noise)
- No API returns a Python list or dict proportional to n or m

**Pointers.** `lab/f2-python/rust/glabpy`.

**Depends on** {{#m2-08-csr-build}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The approved shape of the API and of everything it prints is `design/preview/api.md`; how it renders in a notebook is `design/preview/style/notebook.html`. Issue {{#m0-11-api-sketch}} turns it into the specification this issue implements.
