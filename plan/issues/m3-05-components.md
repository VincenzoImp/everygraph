---
title: Connected components: weak and strong
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P0, risk:low, size:M, lab-evidence
---
**Scope**
- [ ] WCC with Afforest: examined only 2.6% (orkut) and 3.6% (Friendster) of adjacency entries in the lab; hooking by id makes the label the component's minimum node id regardless of thread interleaving, which gives determinism for free
- [ ] One-pass streaming union-find for the semi-external regime (lab: 3.55 s on Friendster under 8 GB), with its O(n) state declared to the planner (the lab's only OOM was this kernel at 1 GB)
- [ ] SCC for directed graphs (parallel forward-backward with trimming, or a parallel coloring variant); needs in-adjacency
- [ ] Outputs: label column, component size table, giant-component mask as a reusable view

**Acceptance**
- orkut WCC ≤ 20 ms at 16 threads; Friendster ≤ 0.5 s in RAM
- Count, largest component and min-id label checksum equal scipy on the oracle graphs
- SCC equals NetworkX on directed oracle graphs and runs on twitter-2010
- Component labels, numbered from 0 by decreasing size with ties broken by the earliest node in graph order (`docs/api-sketch.md` § 2.12), have identical bytes at 1, 4 and 16 threads, with Afforest in memory and with streaming union-find in the semi-external regime, at memory budgets of 2, 8 and 16 GiB and unlimited, and on a view held as a mask or materialized (`design/DECISIONS.md`, 2026-11-25, "What 'repeatable' promises")

**Depends on** {{#m3-01-edge-map}}, {{#m2-11-in-adjacency}}
