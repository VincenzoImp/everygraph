---
title: Seeded relabelling at ingest: does it pay for every kernel, not only out-of-core Leiden?
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P1, risk:medium, size:M, lab-evidence
---
**Question.** Out-of-core Leiden ({{#m1-02-ooc-aggregation}}, PR #143) found that relabelling node ids once at ingest with a seeded bijection (rank of splitmix64(seed ⊕ id)) and then visiting nodes in storage order keeps level-0 local moving at its in-RAM cost under 8 and 4 GiB on Friendster (95.4 s and 107.2 s against 175.3 s and 260.9 s with ADR 0015's block-permuted order), with higher modularity (0.611 against 0.605; official, `bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl`). The independent review found the evidence not yet enough to change the default: one graph, one seed; ids and visiting order changed together; Friendster's ids carry almost no locality, so it is the cheapest case; the default deterministic mode was not measured at small budgets; and relabelling at ingest touches compression (D4), ids and the dictionary (D3, D8) and the locality of every other kernel — it needs its own ADR, not an amendment of D15.

**Method**
- [ ] Through the harness, idle machine: lj, orkut, twitter-sym, Friendster and the web and road families of {{#m1-10-graph-families}}; original ids vs relabelled ids, three seeds
- [ ] Separate the two changes: block order on relabelled ids; storage order on relabelled ids; block order on original ids; id order on original ids
- [ ] Leiden async and deterministic (the default, ADR 0014) at 1 and 3 iterations, in RAM and at 16, 8 and 4 GiB; Q, communities, time per phase, bytes read
- [ ] The cost for the other kernels on relabelled graphs: compressed size (bits per entry, already lj +26%, orkut +16%, Friendster +5%, from the stores' `meta.json`), PageRank, WCC, BFS, k-core, point queries, in RAM and out of core
- [ ] The alternative of a Leiden-only relabelled working copy (built when Leiden needs it), against relabelling the stored graph

**Decision it feeds.** A new ADR: relabel always at ingest, relabel only a working copy for community detection, or keep ADR 0015's order. Whatever is chosen, the visiting order stays a fixed function of the graph and the seed, never of the budget (`design/DECISIONS.md`, "What 'repeatable' promises").

**Depends on** {{#m1-02-ooc-aggregation}}
