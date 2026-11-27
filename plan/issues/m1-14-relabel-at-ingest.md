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
- [ ] The alternative of a Leiden-only relabelled working copy (built when Leiden needs it), against relabelling the stored graph. Preferred before measurement (2026-11-25, `docs/research/sota-storage.md` § 3 m1-14, challenge 5). A random relabelling is the literature's worst case for compression and locality: +4% (Flickr) to +64% (LiveJournal) BV bits on social graphs against the natural order, and 10× on URL-ordered web graphs (LLP Table 6, their data); up to 28.5% application slowdown (DBG paper). Ours: +26% bits on lj, +16% orkut, +5% Friendster
- [ ] A fifth arm, a **seeded locality order**: BP order (seeded, {{#m1-09-hierarchy-order}}) with Leiden visiting in storage order, at 16, 8 and 4 GiB on Friendster and orkut. It measures the opposite of the decorrelation this issue found useful, so whether Leiden keeps its quality and its out-of-core speed is the question

**Constraint (2026-11-25, `design/DECISIONS.md`, "Label propagation follows LDBC CDLP").** Whatever is chosen must keep id-dependent semantics on the original ids: CDLP labels and ties, min-label component outputs, BFS from a given source. Otherwise validation against LDBC Graphalytics fails ({{#m1-20-ldbc-graphalytics}}).

**Decision it feeds.** A new ADR: relabel always at ingest, relabel only a working copy for community detection, or keep ADR 0015's order. Whatever is chosen, the visiting order stays a fixed function of the graph and the seed, never of the budget (`design/DECISIONS.md`, "What 'repeatable' promises").

**Answered by PR #241** ({{#m1-09-hierarchy-order}}, exploratory; sizes and quality do not depend on load): a random relabelling costs 5–31% of storage and slows every kernel, and Leiden's quality does not need it (lj: Q 0.755 with the BP order visited in storage order, 0.750 with block order on the input ids). No relabelling at ingest; ADR 0015's order stays. The reorderings that help are offered by `compact --order` ({{#m2-24-bp-ordering}}).

**Depends on** {{#m1-02-ooc-aggregation}}
