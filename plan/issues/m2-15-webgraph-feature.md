---
title: Optional webgraph feature: BVGraph import/export and archival tier
milestone: M2 · Format and ingest
labels: type:feature, area:interop, P2, risk:low, size:M, lab-evidence
---
**Why.** The LAW collection (the standard large-graph corpus) ships in BVGraph, and on web graphs BV reaches 2.1 bits/edge where byte codecs stay ≥ 9.5. But webgraph-rs decodes at ~100 M arcs/s per core, had four breaking releases in a year and pulls native dependencies that block wasm32 — so it is a feature flag, not the core (DESIGN D5).

**Scope**
- [ ] `--features webgraph`: read `.graph/.properties` (building offsets when missing: 8.3 s for twitter-2010), write BV
- [ ] Optional LLP ordering through webgraph if {{#m1-09-hierarchy-order}} shows our own ordering falls short
- [ ] Pin the dependency version; isolate its generics behind a small adapter (the lab needed a zero-copy `BitFieldVec` workaround to avoid a 19 GB copy)

**Acceptance**
- twitter-2010 BV → `.eg` in < 40 s; exported BV re-imports to an identical graph
- Default build has no webgraph code or dependencies

**Depends on** {{#m2-08-csr-build}}
