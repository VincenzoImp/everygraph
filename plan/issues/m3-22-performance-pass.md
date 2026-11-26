---
title: M3 performance pass: close the measured speed gaps
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P1, risk:medium, size:L
---
**Why.** Under the rule "official runs only when a number is published" (CLAUDE.md § 6, owner 2026-11-26), issues close on correctness and exploratory evidence, and their missed speed targets come here, those of M2 issues included (M2 has no performance pass of its own). All numbers below are exploratory (interleaved A/B on a loaded machine) unless a result file is named.

**Gaps to close**
- [ ] Build (#35): lj 1.30–1.84 s vs ≤ 0.5 s, orkut 5.7–6.2 s vs ≤ 1.0 s (PR #215). Profile on orkut: parsing twice 26 %, finishing buckets 18 %, dictionary collection 13 %, shared atomic item counts 11 %, id look-ups 10 %, scatter 12 %. Friendster targets not yet measured.
- [ ] Edge-map per-task overhead (#44): WCC at 16 threads 1.13× the lab (official, `bench/results/edge-map/20261126T043042Z-c0d7f248bb47.jsonl`): reader slot and scratch handed out once per pass, not per task (PR #224's analysis).
- [ ] BFS (#49): 1.33× GAP on orkut; SSSP 2.06× GAP on orkut, 2.4× on roadnet-ca (PR #220): `NodeSet::list_into` per step (20 %), rayon overhead (14 %), cost-check pass (10 %).
- [ ] k-core (#50): 1.4–1.9× the lab at 1 and 16 threads after the per-round fixes (PR #218): per-entry bucket check and Philox draw per hub edge are the candidates.
- [ ] Compressed BFS/WCC: whole-list decode; batched prefix decode with early exit (#48/#49 follow-up).
- [ ] PageRank (#47): weighted on a per-pair column within 15% of the unweighted kernel on orkut; the out-of-core cost against in RAM from interleaved rounds ({{#m3-19-harness-interleave}}), not from a fixed run order.
- [ ] WCC sampling (#48): first-neighbour against hybrid k-out sampling, interleaved, on orkut, Friendster and a web graph in clustered order (uk-2005, once in the dataset registry).
- [ ] BFS under a memory limit below the adjacency size (#49): Friendster in at most the time of three sequential reads of the graph file.
- [ ] WCC (Afforest) on community orders: 1.2–2.0× slower than on input ids (`lab/m1-09-hierarchy-order/results/README.md`); cause not yet measured.
- [ ] In-adjacency transpose (#38, PR #239): twitter-2010 at 16 threads 1.28–1.50× the lab's raw-array transpose (11.7–38 s at load 18–43, target ≤ 15 s); the candidates are the count pass (random atomic increments) and the scatter. A cap on items per bucket (2^22 tried in PR #239) is a candidate for build rounds held up by one bucket of hubs; it needs an interleaved A/B on two graphs.
- [ ] Strong components (#223, PR #277): BGSS without VGC (local search per task) and hash bags. Exploratory, interleaved, Tarjan time over BGSS time, raw CSR in the heap: at 16 threads web-google 1.69, roadNet-CA 1.20, soc-Epinions1 0.42 (Tarjan faster); at 1 thread 0.17, 0.43 and 0.37. On roadNet-CA the first round's two BFS take about 0.8–1.0 of the BGSS time (exploratory, interleaved). Work: BGSS reads 1.75 m entries in all on twitter-2010 and 4.2 m on web-google, Tarjan m.
- [ ] Planner (#45, PR #249): its choice within 10% of the best manually chosen configuration for PageRank, WCC and BFS on orkut, twitter-sym and Friendster at budgets of 2, 8, 16 GiB and unlimited; first the official `everygraph-bench run edge-map --official --only planner-` that replaces the cost model's exploratory constants
- [ ] Block cache (#178): Leiden level-0 local moving on Friendster under 8 GiB within 1.5× of in RAM (ADR 0029). Exploratory, fast mode, one level, 16 threads, cold: 1.2–1.8× at load 16–44 (the fast mode's sweeps vary from 4 to 6) (`bench/results/block-cache/exploratory/20261126-friendster.jsonl`); deterministic 2.7× (K = 64), with 4.7× the fast mode's bytes. Then the time against per-phase `MADV_RANDOM` on the same run (whole-run `MADV_RANDOM` on the map read 220 GB against the cache's 333 GB, deterministic, 8 GiB: it had about 3.7 GB of page cache against 2.5 GiB of frames), and the deterministic mode's bytes: a frame's records are read again in many sub-rounds of a sweep (ADR 0029, "The cache as built").
- [ ] Same-machine comparison with GAP/GBBS/NetworKit/igraph/GVE (#55's reference runs) decides which gaps matter most.

**Done when.** Each gap is either closed (interleaved A/B against the reference, ratio stated) or its remaining cause is written down with a follow-up; the milestone-end official run records the final numbers.

**Pointers.** `docs/research/sota-kernels.md`; `docs/research/sota-ingest-query.md` challenges 1–2.
