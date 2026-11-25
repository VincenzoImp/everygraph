---
title: API sketch: Python, Rust and CLI from ten user stories
milestone: M0 · Foundations and decisions
labels: type:spec, area:foundations, P0, risk:medium, size:M
---
**Why.** The lab measured mechanisms. The product is the API on top of them, and three lab results constrain it: per-call overhead is 28–62 ns (so the API must be bulk and columnar), an existing `nx.Graph` converts slowly (so native loading must be the easy path), and every long operation needs a preflight estimate.

**Scope**
- [ ] Ten user stories written as runnable-looking code, e.g. "CSV to communities table", "1.8 B edges on a laptop", "neighbors and shortest path of an address", "graph between two dates", "map and share", "keep my NetworkX code"
- [ ] Object model: `Graph` (immutable, file-backed), `View` (filtered), result tables (Arrow), `Hierarchy`, `Map`
- [ ] Naming, defaults (deterministic, physical-core threads, memory budget = fraction of available RAM), error taxonomy
- [ ] Preflight / `explain()` contract: what the engine will do, expected time, RAM and disk
- [ ] CLI verbs mirroring the Python API ({{#m8-01-cli}})
- [ ] Explicit non-goals for v0.1 (mutation, query language)

**Acceptance**
- `docs/api-sketch.md` reviewed against the ten stories by at least one outside graph practitioner
- Each story maps to issues in M2–M7

**Pointers.** FINDINGS § F2 (binding costs, NetworkX traps).

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Start from `design/preview/api.md`, which the owner approved with the recommended options: algorithms are methods on the graph (`g.pagerank()`), results are small Arrow-backed tables carrying the user's original identifiers, repeatable results by default, traversals take `limit=` and `timeout=` ("budget" means memory only), filters are short text expressions, short method names (`components`, `kcore`), the prepared `.eg` file is written next to the input above a size threshold, and long operations print their plan and start without asking (`dry_run=True` only plans). The sketch turns these into a specification; it does not reopen them.
