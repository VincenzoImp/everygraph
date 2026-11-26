---
title: API gaps for v0.1: graph profile, node tables, edge-type name, batch of path pairs
milestone: M6 · Python and interop
labels: type:docs, area:python, P1, risk:low, size:S
---
**Why.** The review of the engines users compare us with (`docs/research/sota-ingest-query.md` § 2.5, § 4 C9) found five things in the approved API that have no name yet. Each is a gap that `docs/api-sketch.md` § 7 already lists, or that its issues name without a public form:
- **Node tables.** Every engine compared has node properties: Kùzu, Neo4j GDS, igraph, graph-tool. #33 already reads node tables; api-sketch § 7 says that "node columns" need a preview.
- **A public name for the edge-type column.** Raphtory calls them layers, GDS relationship types, Kùzu rel tables. #32 and #33 have the reader option without a name.
- **A batch of path pairs** on the planned multi-source BFS kernel ({{#m3-06-traversal}}). DuckPGQ runs bulk many-to-many paths this way. api-sketch § 7: "a batch of pairs … no name here" (#91).
- **A graph profile in one call**, like NetworKit's network profiles: degree, component, core and community size distributions. It mostly assembles kernels v0.1 already has.
- **Bound parameters in filters** (`g.filter(edges="value > $x", params={"x": 10})`, as in GDS), so users do not format numbers into strings.

**Scope**
- [ ] For each of the five: a proposed name and signature, following the naming rule of `docs/api-sketch.md` § 2 (a method is named for what you get), the user story it serves, and the issue that builds it
- [ ] The three that visibly add to what was approved (node tables, the profile, path pairs) get a preview page under `design/preview/` first, in the style of `design/preview/api.md`. The choices are recorded in `design/DECISIONS.md` (CLAUDE.md § 8)
- [ ] `docs/api-sketch.md` § 7 updated: each gap closed with its name, or kept open with the reason
- [ ] Where a building issue is missing, a new issue file (the graph profile has none today)
- [ ] `docs/api-sketch.md` § 6 (every public name, its issue and its story) gains a "Built" column that says which names exist in the package today, added after PR #231 merges (it edits that file); the dogfood pass checks the names it marks as built (decided 2026-11-26)

**Done when.** Each of the five has a decided name, or a recorded decision not to offer it in 0.1, and the api-sketch says so. The decisions feed the review of public names that gates the 0.1 freeze (m8-08, `design/DECISIONS.md` row on #12).

**Pointers.** `docs/research/sota-ingest-query.md` § 2.5 (engines table), § 3 (D21 row), § 4 C9; `docs/api-sketch.md` § 7.

**Depends on** {{#m0-11-api-sketch}}
