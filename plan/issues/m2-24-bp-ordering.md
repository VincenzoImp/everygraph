---
title: Deterministic BP ordering as a compaction step
milestone: M2 · Format and ingest
labels: type:feature, area:format, P2, risk:medium, size:M
---
**Why.** The literature's best ordering for compression is recursive graph bisection (BP; Dhulipala et al., KDD 2016). On BV bits it gains −27% on LiveJournal and −46% on Twitter-sym against the source order (32-core Xeon E5-2660, their data). Our storage keeps the input order only (`docs/research/sota-storage.md` § 3 D4, challenge 1).

BP is too slow for the ingest path: 149 min on Twitter-sym in the paper, about 3.7× less with Mackenzie et al.'s tricks. Promise 1 forbids it there. So it is offered as an opt-in rewrite of the topology outside the ingest (`compact --order`), or deferred. Whether it is worth building is decided by {{#m1-09-hierarchy-order}}'s bake-off. If that experiment shows no gain worth the cost, close this issue with its result.

**Scope**
- [ ] BP with a fixed seed, a fixed depth (log n − 5) and an iteration cap; parallel by subtree, each split seeded and ties broken by id. The output is then the same at any thread count, which the BP paper does not claim; it follows from the independent subproblems (the review's inference) and is tested here
- [ ] A budgeted rewrite: graph in RAM under the accountant, refused with the minimum budget otherwise; time bounded by depth × iterations × m
- [ ] A renumbering permutes the id dictionary as well (D8's rank bitmap for integer keys becomes a 4-byte permutation) and keeps graph order (`docs/api-sketch.md` § 2.2): results are still ordered and tie-broken by the original keys ({{#m2-07-id-mapping}})
- [ ] Leiden's visiting order after a rewrite stays a fixed function of the graph and the seed, never of the budget (D15, `design/DECISIONS.md` "What 'repeatable' promises")
- [ ] The encoder's locality score ({{#m1-09-hierarchy-order}}) tells the user, and the planner's `explain()`, when a reorder is worth offering

**Acceptance**
- Identical output bytes at 1, 4 and 16 threads and at two budgets
- On the graphs of {{#m1-09-hierarchy-order}}, bits/entry and reorder time reported against that experiment's prototype
- Never run by the ingest unless asked for

**Decided after m1-09 (#23, PR #241, exploratory, `lab/m1-09-hierarchy-order/results/README.md`).** The default storage order stays the input order; random ids are never used. `compact --order` offers two orders: the Leiden-hierarchy DFS (−12% to −17% bits on lj, orkut, Friendster; +8% to +12% on twitter-sym, whose input ids are already good) and seeded BP (−8% to −16%, the only one that helps twitter-sym; 74 s lj, 282 s orkut, 9,714 s twitter-sym — budget its time, the top levels dominate). Each is applied only when its log-gap cost is lower than the input's (log-gap cost correlates 0.998–1.000 with our bits). Never inside the ingest. Judge BP against the hierarchy DFS, not the input order.

**Pointers.** `docs/research/sota-storage.md` § 2 (ordering table), § 4 challenge 1, § 5; Mackenzie, Petri, Moffat, SIGIR 2021.

**Depends on** {{#m1-09-hierarchy-order}}, {{#m2-08-csr-build}}
