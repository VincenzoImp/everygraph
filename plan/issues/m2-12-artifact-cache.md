---
title: Derived-artifact cache with provenance
milestone: M2 · Format and ingest
labels: type:feature, area:format, P1, risk:low, size:M
---
**Why.** "Data is written once; everything else is derived" (in-adjacency, raw decode, hierarchy, layout, indexes, PageRank vectors). Results must be reusable across sessions and carry enough provenance to be reproduced — the good part of the old prototype.

**Scope**
- [ ] Artifact key = hash(graph content hash, view definition, algorithm id, parameters, engine version, determinism mode)
- [ ] Store artifacts as sections of the `.eg` file or in a sidecar cache directory (decide; large temporary results should not bloat the graph file)
- [ ] Lookup-before-compute in the planner; `explain()` shows reuse
- [ ] Invalidation rules on engine upgrade (algorithm version numbers, not the whole engine version)
- [ ] `eg info` lists artifacts with size, age and provenance; `eg compact` drops unreferenced ones

**Acceptance**
- Re-running an identical PageRank call returns from cache in milliseconds
- A non-deterministic (`fast`) result is never served for a deterministic request

**Depends on** {{#m2-01-container-spec}}
