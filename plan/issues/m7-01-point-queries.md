---
title: Point queries with budgets and batch variants
milestone: M7 · Queries and views
labels: type:feature, area:query, P0, risk:low, size:M, lab-evidence
---
**Scope**
- [ ] `degrees`, `neighbors` (out/in/both), `has_edge`, `common_neighbors`, `edges_between(set, set)`
- [ ] `khop(v, k, limit=, timeout=)` returning nodes with distance; rows are ordered by distance, then by discovery order — the position of the node's parent in the previous distance's order, then the node's position in that parent's neighbor list (graph order) — and the traversal stops as soon as `limit` rows exist, so its cost is bounded by what it returns (`docs/api-sketch.md` § 2.12); `timeout` bounds wall time; a "truncated" flag in the result
- [ ] Batch forms taking id columns and returning one flat table with a leading `node` column, in input order, `limit` per node (a per-node call from Python costs 28–62 ns, lj, 3 threads, shared machine, `lab/f2-python/results/goalB_lj_dev.json`)
- [ ] External-id resolution in bulk through the dictionary ({{#m2-07-id-mapping}})
- [ ] Decoding straight from the compressed adjacency (random list decode 87–217 ns at 1 thread (#118)) with an optional raw hot tier
- [ ] `khop` with `limit` decodes list prefixes and stops at the budget
- [ ] Property projection on returned edges (weights, timestamps)
- [ ] Kept loops and repeated pairs as `docs/spec/graph-semantics.md` § 6: `has_edge(a, a)` is true exactly when `a` has a kept loop; `neighbors(v)` lists `v` itself once when it has a kept loop, one row per distinct neighbour with `edges` (the multiplicity; the `edges` column under `sum`, 1 under `collapse`), the summed weight, `first`, `last`; `common_neighbors(a, b)` never lists `a` or `b`; `edges_between(A, B)` lists a loop at `v` when `v` is in both sets; reported sums never refuse (§ 4.4)
- [ ] Concurrency: read-only queries from many threads (lab warm throughput: 77–155 M neighbour queries/s, idle, raw, 32 threads (`lab/m1-13-compressed/results/baseline_f1_*.json`))

**Caveat on the baseline.** #118 ({{#m1-13-compressed-end-to-end}}) measured the default storage (idle, 1 thread): neighbours +0.02–0.11 µs per call (2.5–4.2× on timer-free means), bidirectional BFS p50 1.13–1.41×, 3-hop with a 10k budget up to 2.1× (twitter-sym) because whole hub lists are decoded. The acceptance targets are met without the raw tier: orkut neighbours p99 0.48 µs, common neighbours p50 0.52 µs, 3-hop p50 51 µs.

**Acceptance**
- In RAM, single thread: neighbours p99 < 1.5 µs; common neighbours p50 < 1.3 µs; 3-hop with a 10k budget p50 < 0.15 ms on orkut and twitter
- Throughput at 16 threads on the default storage vs raw (neighbours, bidirectional BFS, 3-hop; lj, orkut, twitter-2010, Friendster), held to ADR 0004's 1.5× with the planner's decode tier
- A 2-hop query on a hub returns within its time budget with `truncated = true` instead of running for 15 ms+
- A `khop` cut at `limit` returns the same rows at 1, 4 and 16 threads, and on hub nodes of twitter-2010 and Friendster its time grows with `limit`, not with the size of the hop it stops in

**Where the speed targets went.** The targets missed by the first implementation, and the runs on twitter-2010 and Friendster, are in {{#m7-xx-query-performance}}, with the measured values.

**Pointers.** `lab/f1-query/src/bin/pq.rs`.

**Depends on** {{#m2-08-csr-build}}, {{#m2-10-property-columns}}, {{#m3-02-planner}}
