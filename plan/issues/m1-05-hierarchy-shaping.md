---
title: A hierarchy the map can use: bounded fan-out without losing meaning
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P0, risk:high, size:L, lab-evidence
---
**Question.** How do we get zoom levels with fan-out roughly 8–64 whose regions mean something to the user?

**What the lab found.** Natural Leiden hierarchies shrink 3.5–18× per level but have parents with up to ~100,000 children. A fan-out cap of 64 with level compaction bounds fan-out on all but the top one or two levels, at a cost of 2–5 modularity points, and the median fan-out is only 2–3. The top levels are star-shaped: a few giants each surrounded by hundreds or thousands of small satellite communities. The layout then had to invent "derived regions" (the many identical "627 nodes" bubbles), which look like communities and are not.

**What the tracer found on Friendster ({{#m1-01-e2e-tracer}}, 2026-11-25, idle; `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 9–15, 20–23; `lab/m1-01-tracer/results/b2_louvain_plain_16G.json`, `b2_louvain_capped_56G.json`, `logs/`).** The only partition that fits 16 GiB, plain Louvain (4.2 GiB anonymous, Q 0.6038), has fan-out up to **18,723,769** children, and the lab layout cannot use it even with 56 GiB (killed during depth 3, 55.8 GiB anonymous). The cap applied during coarsening makes coarsening shrink slowly — level 1 has 10.5 M communities instead of plain Louvain's 1.16 M, and the capped Leiden's pass 3 still has 1.42 B entries — and under 16 GiB both capped runs are killed in pass-0 aggregation. At 56 GiB, keeping every level's quotient graph (`keep_graphs` with `--out-hier`) is what kills the capped Leiden: about 51.5 GiB of quotient graphs by arithmetic (2.77 + 2.03 + 1.42 + 0.70 B entries × 8 B; not tested without `--out-hier`). The capped hierarchy itself works for the map (Louvain: fan-out ≤ 64 on four of six levels, then 1,094 and 6,478; Q 0.5757; layout 69.6 s). **Shaping must therefore be bounded in memory.**

**Now measured by {{#m1-02-ooc-aggregation}} (2026-11-25, idle, PR #143, FINDINGS § M1-02).** Map-shaped Leiden (cap 64, compaction 3; async, 1 iteration, 16 threads, f64 weights, compressed level 0) runs on Friendster under `MemoryMax=16G`: 164.1 s, anonymous peak 10.9 GiB, Q 0.5900, **27 top communities**, 0 disconnected, 99.2 GB read (`bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl`, case `ooc-friendster-map/limit=16G/t16`). Its levels, from the matching `.program.jsonl`: 20,931,249 / 4,971,728 / 1,130,490 / 318,223 / 62,942 / 2,027 / 322 / 27 communities, maximum fan-out 64 on the four lowest levels, then 564, 549, 89 and 129: as in the lab, the cap does not bound the top levels. It uses sequential aggregation with every level spilled as a compressed store, so the memory bound above no longer rules out shaping during coarsening. This hierarchy is the Friendster input for the candidates below; no in-RAM reference of it exists, and no layout has been run on it yet.

**Candidates**
- [ ] Cap applied after an uncapped solve: shape the plain partition and its quotient graphs in a post-pass, so community detection keeps the plain run's memory (4.2 GiB on Friendster under 16 GiB) and its fast-shrinking levels
- [ ] Top-down recursive sub-clustering of oversized communities on the exported quotient graphs (not tested in the lab); on Friendster that means communities of up to 19.8 M nodes, one at a time within the budget
- [ ] Nested resolution levels: run CPM/modularity at increasing γ *inside* each parent so levels nest by construction (plain γ sweeps are not nested)
- [ ] Satellite handling: an explicit "N small communities" aggregate per giant instead of fake regions
- [ ] Keep the cap approach as baseline; fix the known bug (`--cap` ignored after the first iteration)

**Metrics**
- Fan-out distribution per level (median, p95, max, share in 4–64)
- Modularity of each level vs the uncapped partition; conductance of regions
- Share of nodes living in "derived" (non-community) regions
- Stability: does a small perturbation of the graph reshuffle the top levels?
- Visual check on lj, orkut and one non-social graph through the lab layout
- Peak anonymous memory, time and bytes read of shaping on Friendster under `MemoryMax=16G`, against the tracer's plain solve alone (4.2 GiB, 115.1 s; `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 15)

**Decision it feeds.** D16 and the artifact format in {{#m4-07-map-hierarchy}}.

**Done when.** One method is chosen with numbers on three graphs, and the map screenshots no longer contain unlabeled artificial regions; on Friendster the chosen method fits `MemoryMax=16G` and its hierarchy has no parent the layout cannot place.
