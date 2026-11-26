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
- [ ] The whole-graph algorithms through `planner::plan` and `run::execute` ({{#m3-02-planner}}): `Plan` from the plan document, the refusal's `smallest_budget_bytes`, and one `run::Session` per `Graph`, so that the decode tier's raw copy is kept between traversals and counted in every later plan
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

**Left after PR #229 (2026-11-26).** PR #229 built the first slice (`crates/everygraph-py`): `eg.read` of text edge lists, `eg.open`, `Graph` with `pagerank`, `bfs`, `components`, `kcore`, `degrees`, `top`, Arrow-backed `Table`s, the exception classes, and progress events; its description lists the evidence per box. Still open:
- [ ] The user stories' steps that need other issues: communities ({{#m4-01-leiden-production}}), Parquet ({{#m2-06-parquet-reader}}), views ({{#m7-04-views}}), point queries ({{#m7-01-point-queries}}, {{#m7-02-shortest-paths}}), `from_edges` ({{#m6-04-in-memory-constructors}}), weight and time columns ({{#m2-34-build-columns}}), directed graphs' in-edges ({{#m2-11-in-adjacency}})
- [ ] PageRank from Python within 3% of the native binary: needs `everygraph run pagerank` ({{#m8-01-cli}}); an interleaved A/B closes it (CLAUDE.md § 6)
- [ ] `explain()`, `dry_run`, and the `plan`, `result` and `error` events of `docs/ux-contract.md` G-16 (one `plan` first, one `result`, `stopped` or `error` last): they come with the planner ({{#m3-02-planner}}); today only `phase` and `stopped` are sent
- [ ] Cancellation inside a kernel (`docs/ux-contract.md` C-4): the engine has none, so `Ctrl-C` is raised when the kernel ends

Disagreements with `docs/api-sketch.md` and `docs/ux-contract.md` found by PR #243 (decided 2026-11-26: the documents win; the code is fixed in a later Python pull request):
- [ ] A missing or unknown column, a missing header or a column beyond the header raises `InvalidInput` (an `eg.EverygraphError`, exit 3; the table of `docs/api-sketch.md` § 2.7 gives it no `ValueError` base), not a plain `ValueError` (`crates/everygraph-py/src/error.rs`)
- [ ] OS errors keep their subclass (`FileNotFoundError`, `PermissionError`, …; `docs/api-sketch.md` § 2.7), not a generic `OSError` made with `io::Error::other`
- [ ] Default threads are the physical cores the process may use (`docs/api-sketch.md` § 2.4; D13), not `available_parallelism()`; the planner ({{#m3-02-planner}}) later chooses per kernel
- [ ] The other gaps of PR #243's list (messages, the stop block, the cache-hit notice, `Unsupported` cases no document lists), each fixed in the code or the document. `smallest_budget_bytes = None` without the planner is settled by the amendment in PR #231

**Depends on** {{#m2-08-csr-build}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The approved shape of the API and of everything it prints is `design/preview/api.md`; how it renders in a notebook is `design/preview/style/notebook.html`. Issue {{#m0-11-api-sketch}} turns it into the specification this issue implements.
