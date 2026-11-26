---
title: Derived-artifact cache with provenance
milestone: M2 · Format and ingest
labels: type:feature, area:format, P1, risk:low, size:M
---
**Why.** "Data is written once; everything else is derived" (in-adjacency, raw decode, hierarchy, layout, indexes, PageRank vectors). Results must be reusable across sessions and carry enough provenance to be reproduced — the good part of the old prototype.

**Scope** (the store and its look-up in the format; the callers that use it are scoped
in their own issues, as listed below)
- [ ] Artifact key = graph identity of `docs/spec/eg-container.md` § 6, algorithm id and version, determinism mode and every parameter that can change the result; never the memory budget or the thread count (`docs/api-sketch.md` § 2.8). Results on a view are not stored in the graph file, so the key has no view definition
- [ ] Store artifacts as sections of the `.eg` file. Decided: results on the whole graph live inside the graph file, and there is no hidden cache directory (`docs/api-sketch.md` § 2.8; DESIGN D7, ADR 0027); `compact` reclaims what removed results leave
- [ ] Invalidation on engine upgrade by algorithm version numbers, not the whole engine version: the version is part of the key
- [ ] A typed result section with its provenance record, specified in `docs/spec/eg-container.md` § 6 and checked by the independent Python reader (identity)
- Moved to the callers (2026-11-26, when the store was built and no kernel existed yet): lookup-before-compute and `explain()` showing reuse to {{#m3-02-planner}}; `everygraph info` listing results and `everygraph compact`, and the `map` phases committed as cached results (`docs/ux-contract.md` § 5.1, C-2), to {{#m8-01-cli}}; "an identical PageRank call returns from cache in milliseconds" to {{#m3-04-pagerank}}

**Acceptance**
- A non-deterministic (`fast`) result is never served for a deterministic request
- A result is served only under its own key and for the graph it was computed on; a crash while storing or replacing one leaves no result, the old one or the new one, whole (the container's crash-injection tests)

**Depends on** {{#m2-01-container-spec}}
