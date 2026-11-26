---
title: Server-class scale-up: 64–128 cores, NUMA and the whole map pipeline
milestone: M9 · After v0.1
labels: type:feature, area:engine, P1, risk:medium, size:L
---
**Why.** Everygraph should make the most of the hardware it has, small or large (owner, 2026-11-26). Today no run uses more than 32 threads, nothing is NUMA-aware, and m9-07's acceptance covers ingest, WCC and PageRank, not the map.

**Scope**
- [ ] Strong and weak scaling of the kernels, Leiden and the map pipeline at 16, 32, 64 and 128 cores on a rented server, with parallel efficiency and memory bandwidth reported
- [ ] NUMA-aware allocation and scheduling where the scaling runs show it pays (with m9-07)
- [ ] The whole map pipeline (ingest → hierarchy → layout → tiles) on a ≥ 10 B-edge graph, with time, peak, bytes read and artefact size
- [ ] Serial phases named and measured (Leiden aggregation, hierarchy shaping)

**Depends on** {{#m9-07-u64-ids-server-scale}}, {{#m5-03-tile-builder}}
