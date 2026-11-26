---
title: Connected components: weak and strong
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P0, risk:low, size:M, lab-evidence
---
**Scope**
- [ ] WCC as ConnectIt's UF-Rem-CAS finish with **hybrid k-out sampling**: each node's first neighbour plus one random neighbour chosen by a counter-based hash of (seed, node), no sampling when m/n < 3 (decided 2026-11-25, `docs/research/sota-kernels.md` § 3–4, challenge 6). Not first-neighbour sampling alone (the lab's Afforest, which examined only 2.6% (orkut) and 3.6% (Friendster) of adjacency entries): that is the "kout-afforest" variant ConnectIt shows failing on locally clustered orders (ClueWeb, Hyperlink2012), which is what a community or locality order ({{#m1-09-hierarchy-order}}, {{#m1-14-relabel-at-ingest}}) makes of any graph. ConnectIt reports 1.67× (Friendster) to 8.6× (orkut) over GAP Afforest (VLDB'21, 4 × 18-core E7-8867 v4, 144 hyper-threads, their machine). Linking to the smaller root keeps the label the component's minimum node id regardless of thread interleaving
- [ ] Interleaved A/B before choosing: orkut, Friendster and uk-2005 (a web graph in clustered order) at 16 threads, first-neighbour vs hybrid sampling; time and label hash
- [ ] One-pass streaming union-find for the semi-external regime (lab: 3.55 s on Friendster under 8 GB), with its O(n) state declared to the planner (the lab's only OOM was this kernel at 1 GB)
- [ ] WCC on the compressed source with batched prefix decode, measured on lj, orkut, twitter-sym, Friendster at 1 and 16 threads against raw (`./bench.sh build encode followup_wcc` in `lab/m1-13-compressed`, about 10 min, an estimate). #118's per-node prefix path is 1.36× (16 threads) and 1.57× (1 thread) of raw on Friendster, 1.43× on orkut at 16 threads
- [ ] SCC for directed graphs with **BGSS** (Wang, Dong, Gu, Sun, SIGMOD'23): random-permutation prefix-doubling multi-reachability, a deterministic phase-concurrent hash table, vertical granularity control; then one O(n) pass that relabels each SCC to its minimum id. Not forward-backward with trimming or coloring: that is the Multistep/iSpan family, which the BGSS paper measures slower than sequential Tarjan in 24 of 36 large-diameter tests; BGSS reports 1.7× (Twitter) and 3.1× (LJ) over GBBS and 4–19× on large-diameter graphs (4 × 24-core Gold 6252, 192 hyper-threads, their machine). Needs in-adjacency (decided 2026-11-25, `docs/research/sota-kernels.md` challenge 7)
- [ ] Outputs: label column, component size table, giant-component mask as a reusable view
- [ ] Out of core, WCC may skip inactive lists and run in any order: the answer is the unique fixpoint of a monotone update, so the output is identical by construction (Seraph-style active-list decoding, Gem-style bound pruning; {{#m3-03-semi-external}}; `docs/research/memory-bounded-algorithms.md` addendum § F 5, added 2026-11-25). Identity with the in-RAM result is asserted in the tests

**Acceptance**
- orkut WCC ≤ 20 ms at 16 threads; Friendster ≤ 0.5 s in RAM (default storage measured 17.2 ms (orkut) and 0.497 s (Friendster) in #118)
- Count, largest component and min-id label checksum equal scipy on the oracle graphs
- SCC equals NetworkX on directed oracle graphs and runs on twitter-2010
- Component labels, numbered from 0 by decreasing size with ties broken by the earliest node in graph order (`docs/api-sketch.md` § 2.12), have identical bytes at 1, 4 and 16 threads, with the in-memory sampling kernel and with streaming union-find in the semi-external regime, at memory budgets of 2, 8 and 16 GiB and unlimited, and on a view held as a mask or materialized (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises")

**Left after PR #218 (2026-11-26).** PR #218 built `everygraph_kernels::components` (WCC with hybrid k-out sampling and a one-pass variant); its description lists the evidence per box. Closed on 2026-11-26 on its tests and exploratory evidence (CLAUDE.md § 6). Where the rest went: SCC with BGSS, {{#m3-20-scc}}; the one-pass kernel declared to the planner, {{#m3-02-planner}}; WCC on the compressed source and the first-neighbour against hybrid A/B (uk-2005 is not in the dataset registry yet), {{#m3-22-performance-pass}}; identity on a view, {{#m7-04-views}}. The official times come from the M3 milestone window.


**Depends on** {{#m3-01-edge-map}}, {{#m2-11-in-adjacency}}
