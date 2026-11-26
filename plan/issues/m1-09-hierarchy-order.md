---
title: Renumber nodes by community hierarchy: compression and speed for free?
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:format, P1, risk:low, size:M, lab-evidence
---
**Question.** LLP ordering improved BV compression on orkut by 26% (15.25 → 11.32 bits/arc), varint by 18%, Stream VByte by 14%, and made raw PageRank 32% faster — but LLP took ~9 minutes. A flat community id gained almost nothing (−3%). Does a lexicographic key over *all* levels of the Leiden hierarchy, which the product computes anyway, get close to LLP?

**What the literature says** (widened 2026-11-25, `docs/research/sota-storage.md` § 2–4, challenges 1, 5, 7; paper numbers, their machines). Everyone who reports sizes reports them after an ordering; ours is input order only, the largest gap in D4.
- BP (recursive graph bisection, KDD 2016, 32-core Xeon E5-2660) is best on all but one graph: BV bits LiveJournal 14.61 → 10.73 and Twitter-sym 21.56 → 11.62. It took 3 min on LiveJournal and 149 min on Twitter; Mackenzie et al. report 3.7× faster with their median-finding tricks. It is deterministic if each split is seeded and ties break by id (inference).
- LLP comes second.
- Random ids are the standard worst case: +64% BV bits on LiveJournal against the natural order (LLP Table 6). Speed-ups reported against random ids (Rabbit Order, BOBA) overstate what real inputs gain.
- The average log gap predicts compressed size: correlation 0.968 with BV bits/link (LLP).

**Method.** Through the harness on an idle machine. Every run that loads a graph runs under `systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0`, except the BP and LLP runs, which need the graph in RAM: their limit is stated with the run.
- [ ] Graphs: lj, orkut, twitter-sym, Friendster, uk-2005
- [ ] Orderings: original; random (seeded); BFS; DBG; the Leiden-hierarchy DFS order (with the flat-community and hierarchy-path-plus-degree/BFS variants as before); BP with a fixed seed, fixed depth (log n − 5) and an iteration cap (Petri–Mackenzie code in `lab/`); LLP (webgraph CLI)
- [ ] Per ordering × graph: our bits/entry (index included) and BV bits; the log-gap cost; the time to compute the order and to permute and re-encode; PageRank, BFS and WCC time as interleaved A/B in one process at 16 threads; Leiden time and modularity
- [ ] A locality score in the encoder's stats (challenge 7): the mean log2 gap per entry, and the ratio of bits/entry to the random-list bound 2 + log2(n/d). By the review's arithmetic on our stores the ratio is about 0.86 (lj), 0.95 (orkut), 0.74 (twitter-sym) and 1.04 (Friendster). Check that it ranks the graphs by the gain the orderings measure
- [ ] For the layout ({{#m1-07-layout-area}}, `docs/research/sota-maps.md` challenge 4): Friendster under `MemoryMax=16G`, the lab layout on ids in hierarchy (DFS) order against the original ids. Bytes read and time. In DFS order each parent's members are a contiguous id range and every depth is one sequential pass. The review's arithmetic, an extrapolation: about 8 × 15.5 GB ≈ 124 GB sequential, against 733 GB of random re-reads in the tracer

**Decision it feeds.**
- Whether Everygraph renumbers after community detection by default, and whether the layout order and the storage order should be the same permutation ({{#m2-03-adjacency-codec}}).
- Whether a deterministic BP ordering is offered as a compaction step ({{#m2-24-bp-ordering}}), never inside promise 1's ingest.
- The baseline order for block codec 2 ({{#m1-16-codec-0124}}) and for judging the archival tier ({{#m2-20-archival-codec}}).
- The default against random relabelling in {{#m1-14-relabel-at-ingest}}.

**Done when.** A table ordering × graph × {bits/entry, log-gap cost, reorder time, PageRank / BFS / WCC time}, the locality score's ranking, and a yes/no on default renumbering.
