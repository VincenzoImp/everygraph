---
title: Point queries with budgets and batch variants
milestone: M7 · Queries and views
labels: type:feature, area:query, P0, risk:low, size:M, lab-evidence
---
**Scope**
- [ ] `degree`, `neighbors` (out/in/both), `has_edge`, `common_neighbors`, `edges_between(set, set)`
- [ ] `khop(v, k, limit=, timeout=)` returning nodes with distance; rows are ordered by distance, then by discovery order — the position of the node's parent in the previous distance's order, then the node's position in that parent's neighbor list (graph order) — and the traversal stops as soon as `limit` rows exist, so its cost is bounded by what it returns (`docs/api-sketch.md` § 2.12); `timeout` bounds wall time; a "truncated" flag in the result
- [ ] Batch forms taking id columns and returning one flat table with a leading `node` column, in input order, `limit` per node (per-node calls cost 30–60 ns from Python)
- [ ] External-id resolution in bulk through the dictionary ({{#m2-07-id-mapping}})
- [ ] Decoding straight from the compressed adjacency (random list decode 0.3–0.4 µs in the lab) with an optional raw hot tier
- [ ] Property projection on returned edges (weights, timestamps)
- [ ] Concurrency: read-only queries from many threads (lab warm throughput at 32 threads: 34–72 M neighbour queries/s)

**Caveat on the baseline.** The lab latencies were measured on *uncompressed* arrays. Decoding a list from the default compressed format costs 0.3–0.4 µs against ~0.1 µs raw, and more for hub lists, so these targets need either the raw hot tier or re-measurement ({{#m1-13-compressed-end-to-end}}).

**Acceptance**
- In RAM, single thread: neighbours p99 < 1.5 µs; common neighbours p50 < 1.3 µs; 3-hop with a 10k budget p50 < 0.15 ms on orkut and twitter
- A 2-hop query on a hub returns within its time budget with `truncated = true` instead of running for 15 ms+
- A `khop` cut at `limit` returns the same rows at 1, 4 and 16 threads, and on hub nodes of twitter-2010 and Friendster its time grows with `limit`, not with the size of the hop it stops in

**Pointers.** `lab/f1-query/src/bin/pq.rs`.
