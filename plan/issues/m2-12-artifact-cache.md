---
title: Derived-artifact cache with provenance
milestone: M2 · Format and ingest
labels: type:feature, area:format, P1, risk:low, size:M
---
**Why.** "Data is written once; everything else is derived" (in-adjacency, raw decode, hierarchy, layout, indexes, PageRank vectors). Results must be reusable across sessions and carry enough provenance to be reproduced — the good part of the old prototype.

**Scope**
- [ ] Artifact key = hash(graph identity of `docs/spec/eg-container.md` § 6, view definition, algorithm id, parameters, engine version, determinism mode)
- [ ] Store artifacts as sections of the `.eg` file. Decided: results on the whole graph live inside the graph file, and there is no hidden cache directory (`docs/api-sketch.md` § 2.8; DESIGN D7, ADR 0027); `compact` reclaims what removed results leave
- [ ] Lookup-before-compute in the planner; `explain()` shows reuse
- [ ] Invalidation rules on engine upgrade (algorithm version numbers, not the whole engine version)
- [ ] `everygraph info` lists artifacts with size, age and provenance; `everygraph compact` drops unreferenced ones
- [ ] The `communities`, `hierarchy` and `layout` phases of `map` are committed as cached results as each finishes, so a stopped `map` keeps them and a later run shows them as `kept` (`docs/ux-contract.md` § 5.1, C-2). Results computed on a view are not stored in the graph file: they live for the session and are kept only when written out (`design/DECISIONS.md` via `docs/api-sketch.md` § 2.8)

**Acceptance**
- Re-running an identical PageRank call returns from cache in milliseconds
- A non-deterministic (`fast`) result is never served for a deterministic request

**Depends on** {{#m2-01-container-spec}}
