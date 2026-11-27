---
title: M7 epic: queries and views
milestone: M7 · Queries and views
labels: type:epic, area:query, P0
---
Tracking issue for milestone M7.

**Goal.** Interactive questions — neighbours, k-hop, how are A and B connected, only edges in March above a threshold — answered in micro- to milliseconds in RAM and usefully out of core, with budgets and timeouts on everything.

**Lab baseline (warm, single thread, p50).** degree/neighbours 0.1–0.6 µs; common neighbours 0.4–0.8 µs; 3-hop with a 10k-node budget 56–115 µs; bidirectional BFS 0.3 ms (lj), 0.6 ms (orkut), 1.5 ms (twitter) on a shared machine; idle (#118, graph in heap) 0.13 / 0.34 / 0.47 ms on raw arrays and 0.16 / 0.45 / 0.56 ms on the default storage (lj / orkut / twitter-2010 directed). For comparison on lj: Neo4j shortest path 6.4 ms, embedded engines 0.3–4.8 s. Cold: neighbours 0.1–0.25 ms; shortest path 25 ms on 10^8 edges, 0.8 s on Friendster. Tails on hub-heavy graphs: p99 up to 15 ms (2-hop) and 113 ms (path) even in RAM (shared machine; idle, #118, twitter-2010 path p99: 32.3 ms raw, 27.3 ms default storage).

**Exit criteria**
- [ ] {{#m7-01-point-queries}}, {{#m7-02-shortest-paths}}, {{#m7-03-ooc-queries}}
- [ ] {{#m7-04-views}}, {{#m7-05-temporal}}
- [ ] {{#m7-06-duckdb-arrow}}; optional {{#m7-07-distance-index}}
- [ ] {{#m7-09-subgraph-bridge}}, {{#m7-10-tutorials}}
- [ ] On the map: live mode {{#m5-08-live-mode}}; optional overlays {{#m5-13-attribute-overlays}} (both need the viewer from M5)
- [ ] Neighbours < 1.5 µs p99 in RAM and < 0.3 ms p50 cold; shortest path p50 < 2 ms in RAM on twitter-2010
- [ ] Milestone close (CLAUDE.md § 6, § 10): the dogfood pass, the cleanup and test audit, the official measurement window, and the report `docs/reports/M7.md`. Its issues are filed when the last P0 of the milestone starts

**Not in v0.1:** a query language (see {{#m9-03-gql-subset}}).
