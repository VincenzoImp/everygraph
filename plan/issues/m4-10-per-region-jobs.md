---
title: Map hierarchy: each oversized region solved as its own job
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:medium, size:M, lab-evidence
---
**Why.** The map's recursion (`rec`, chosen by {{#m1-05-hierarchy-shaping}}, PR #225) re-clusters every oversized region of a round, but it solves all of that round's regions in one Leiden run. They share the visiting order and Leiden's global stopping rules. So a region solved alone and the same region in the union of its round give different sub-partitions: ARI 0.69–0.99, and 0.34 on lj's 589,084-node giant against 0.64 for the same region alone with another visiting order (PR #225, `lab/m1-05-hierarchy-shaping/results/independence.jsonl`, exploratory-deterministic).

Under the nesting constraint, the objective decomposes by region: modularity with full-graph degrees, the global m and a per-region resolution, as now, or CPM, which is unchanged on subgraphs (Traag et al. 2011). Solving each region alone therefore makes "region alone = region in the union" hold by construction. graspologic's `hierarchical_leiden` does this, serially (`docs/research/sota-communities.md` § 3 D16, § 4 challenge 4).

**Built by PR #285 ({{#m4-07-map-hierarchy}}).** Each region is solved alone, by Leiden on its own induced subgraph with its own visiting order and stopping test; regions below 2^22 entries run several at a time, larger ones alone; results are committed in region order. So "region alone = region in the union" holds by construction, and the tree is the same at any thread count (`tests/map.rs`). Left: the seed per region and the measurements below.

**Scope**
- [ ] Each oversized region is one job, with seed = hash(seed, region id), its own visiting order and its own stopping test
- [ ] Small regions batched into one job by a size threshold that depends on the region, never on the budget or the thread count; one large region at a time bounds memory
- [ ] Jobs scheduled in parallel, results committed in region order, so the output is identical at any thread count and budget
- [ ] The objective inside a region stays the one of {{#m4-07-map-hierarchy}} (full-degree null model); CPM as an option, from {{#m4-02-weights-resolution}} (PR #294)

**Acceptance**
- Rerunning PR #225's independence check (`dev_independence.py`) on lj and web-google gives identical sub-partitions for a region alone and in the union
- The fan-out table and the T+1 stability of #225 reported against `rec`; time and memory on Friendster under `MemoryMax=16G`
- Identical hierarchy hash at 1, 4 and 16 threads

**Pointers.** PR #225, "Prior art and follow-ups" and "Independence of a round"; `docs/research/sota-communities.md` § 4 challenge 4; graspologic-native `leiden/hierarchical.rs`.

**Depends on** {{#m1-05-hierarchy-shaping}}, {{#m4-01-leiden-production}}
