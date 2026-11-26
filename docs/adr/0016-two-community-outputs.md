# ADR 0016 — Two community outputs, one artifact: the partition T inside the map tree

- **Status:** accepted
- **Decision:** D16 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25; amended by #19 on 2026-11-26 and by #63 on 2026-11-26

## Context

Users want the best partition (for analysis and papers). The map wants zoom levels with a
bounded number of children per region (for layout and tiles). One natural hierarchy does
not serve both: its parents have up to hundreds of thousands of children (evidence of
2026-11-25, below). #19 compared five ways to shape a map hierarchy (FINDINGS § M1-05).

## Decision

- Community detection produces the partition T (the best modularity found) and a map tree
  built from T.
- **T is a flagged antichain of the tree.** Every region records its role. The regions
  flagged "partition" are exactly T's communities, at whatever depth. The analysis output
  is T, read from those flags; it is never a depth of the tree.
- **Below T: recursive sub-clustering.** Every region above 64 nodes is split again by
  Leiden on its own induced subgraph, with its own null model (local modularity, γ = 1),
  until regions hold at most 64 nodes or cannot be split.
- **Above T: a super level** when T needs more than 63 communities to cover 90% of the
  nodes.
  - It is Leiden on the whole graph at the largest resolution 2^-j whose groups need at
    most 63 communities to cover 90% of the nodes.
  - Each community of T goes to the group that holds most of its nodes, so the levels nest.
  - When a dataset's map is rebuilt: built above 72, removed below 56, otherwise the
    previous decision is kept (`design/DECISIONS.md`, "The map's super level"). This rule
    is a heuristic; no prior art was found.
- **Tails.** A region with more than 64 children keeps its 63 largest children and one
  aggregate. The map draws the aggregate as one hatched area, "+N smaller communities".
- **Drawing aids.** Aggregates, subdivisions and layout-derived groups are drawing aids.
  They are drawn and labelled as such and never reported as communities.

## Evidence

Measured by #19 (FINDINGS § M1-05). Official rows are in `bench/results/hierarchy-shaping/`. The other records are
exploratory-deterministic (shared machine, deterministic metrics) under
`lab/m1-05-hierarchy-shaping/results/`.
- **Fan-out** is at most 64. Parents with 4–64 children:
  - Friendster: 99.7% (official, `20261125T221248Z-4efe4bc4defe.jsonl` row 7);
  - lj, orkut, web-google, roadnet-ca: 90.7–99.9%.

  The cap baseline has median fan-out 3 and 38–42% of parents in band on lj, orkut,
  web-google and Friendster.
- **Leaves and aggregates.** Leaf communities have a median of 13–20 nodes. At most 10.2% of the
  nodes are under an aggregate (Friendster 2.1%).
- **T is kept exactly.** The modularity of T's level equals the plain partition's on every
  graph. The cap baseline's top partition loses 1.6 points on Friendster, 1.9 on lj and
  4.3 on orkut.
- **Friendster under `MemoryMax=16G`** (official, `20261126T035728Z-157b423f2463.jsonl`
  rows 1–2):
  - shaping after the solve: 499.3 s and 506.1 s, anonymous peak 10.0 GiB, 9 rounds, 128.6 GB read;
  - the plain solve before it: 102.2 s and 113.4 s, 7.4 GiB.
- **Layout.** The lab layout places every parent of the Friendster tree under 16 GiB
  (64.7 s, `20261125T221248Z-4efe4bc4defe.jsonl` row 10).
- **The other candidates** do worse:
  - `sat` (natural hierarchy plus aggregates) and `capafter` (cap after the solve) keep
    median fan-out 3, and put up to 75.4% of the nodes under aggregates;
  - nested resolution levels split into leaves of 1–3 nodes.

## Caveats

- **Levels above T** are low-resolution groupings nested by majority vote. Their drawn
  modularity is lower than T's: web-google 0.9666 against 0.9775 and roadnet-ca 0.9812
  against 0.9923, where the rule builds them. An always-built super level would cost 2.5
  points on lj, 14.7 on orkut and 7.5 on Friendster.
- **Levels below T** are local-modularity sub-clusters at γ = 1.
  - They are weak as partitions of the whole graph. Q at T+1 / T+2: lj 0.62 / 0.47,
    orkut 0.58 / 0.40, Friendster 0.55 at T+1. Node-weighted conductance at T+1: 0.36–0.43
    on the social graphs.
  - They are less stable than T. Under six 1% edge drops, ARI at T / T+1 / T+2 is
    lj 0.754 / 0.437 / 0.329 and orkut 0.831 / 0.554 / 0.376. NMI is 0.80–0.87 below T.
  - A round solves all its regions in one Leiden run, so they share the visiting order and
    the global stopping rules. A region solved alone gives ARI 0.69–0.99 against the same
    region in the union of its round (0.34 on lj's 589,084-node giant). Follow-up #181
    (m4-10) solves each region as its own job.
- **The super level switches on discontinuously** at its threshold. Margins measured: lj 14
  below, orkut 53 below, Friendster 57 below, web-google 28 above, roadnet-ca 256 above.
  - The decision did not change under 12 perturbed graphs, once Leiden's stopping test
    ignores vertices of strength 0.
  - Without that rule, a 1% drop on lj stops the plain solve one pass early. lj then needs
    82 instead of 49 communities to cover 90% of the nodes (Q 0.7450 instead of 0.7477;
    `stopfix.jsonl`). #57 carries the rule.
- **Leaves that cannot be split.** Star-shaped regions above 384 nodes stay single leaves:
  lj 3 leaves with 1,961 nodes, web-google 6 with 4,004, Friendster 8 with 4,242. The lab
  layout draws them with 99 derived groups on Friendster. Follow-up #182 (m4-11) replaces
  those with labelled binning in the tree.
- **T's own instability.** The map inherits it: ARI 0.754 on lj and 0.831 on orkut under
  1% drops (deterministic Leiden, one iteration).
- **The 2026-11-25 condition** ("finest level within one modularity point of the best
  partition") holds only in this sense: T is a level of the tree. The drawn top level is
  lower wherever it holds an aggregate or a super level.

## Consequences

- The partition is the analysis result, read from the tree's flags. The other regions of
  the map tree are never reported as "the communities".
- The map tree is stored as the levels of a hierarchy by depth, a leaf repeated on each
  level below its own, with the kind of every region (`docs/spec/eg-communities.md` § 6;
  amended by #63). The roles are read from the kinds; an aggregate's count and whether it
  is nested are read from the tree. So the exact counts, the quotient graphs and their
  checks serve the map unchanged.
- The production Leiden (#57) leaves vertices of strength 0 out of its pass-stopping test.
- In production (#63), each oversized region is solved alone, by Leiden on its own
  induced subgraph: so a region alone and the same region in the union of its round give
  the same split by construction. Small regions are solved several at a time, a region of
  2^16 nodes or more by all threads, one at a time.
- The super level's Leiden runs coarsen while a level shrinks the graph by 1% or more, up
  to 60 levels, as the lab's `upward` did (`--agg-tol 0.99`). With the usual rule of the
  production Leiden, roadnet-ca reached no cover of 63 down to `2^-16` and its drawn top
  level fell to Q 0.728; with this rule, 0.9815 at `2^-4` (exploratory, PR for #63).
- Follow-ups: #181 (m4-10, what is left of per-region jobs: seeds per region, the
  independence and stability runs), #182 (m4-11, labelled binning for leaves that cannot
  be split).

## What would change our mind

- Per-region jobs (#181) still leave the levels below T less stable than the alternatives
  at the same fan-out. Then the map needs another objective below T (CPM, or a coarser
  null model).
- A graph where recursive sub-clustering cannot reach fan-out 4–64 for most parents.

## Original decision and evidence (2026-11-25)

### Decision (2026-11-25)

Community detection produces two outputs: the partition with the best modularity, and a
map hierarchy shaped for the map (fan-out capped, levels compacted).

### Evidence (2026-11-25)

Idle, 16 threads, async Leiden unless noted (FINDINGS § B2 "Hierarchy for the map";
`lab/b2-leiden/results/hier/`, "Hierarchies (whole machine)" in
`lab/b2-leiden/results/summary.txt`):
- **Natural hierarchies are unusable for a map**: the largest parent has 102,821 children
  on lj and 338,769 on orkut (level 1, compaction ratio 3). DESIGN and FINDINGS said
  "~100k" until on 2026-11-25; the idle orkut run is 3.4× that. Louvain is shallower still (orkut 3.07 M →
  11,044 → 118 → 37 → 33 nodes per level).
- **The cap costs modularity**: fan-out cap 64 with compaction ratio 3 lowers top-level
  modularity from 0.7494 to 0.7260 on lj (2.3 points) and from 0.6795 to 0.6297 on orkut
  (5.0 points). Fan-out is ≤ 64 on all but the top one or two levels (lj top level: max
  1,477 children); median fan-out is only 1–3 on most levels.
- **The map shows the failure** (FINDINGS § E; shared; `lab/e-map/REPORT.md`): the layout
  had to invent derived regions (identical "627 nodes" bubbles) and the top levels are
  star-shaped. A fallback hierarchy made the layout's median edge-length ratio 3× worse
  (`lab/e-map/results/eval_lj_hcl_lpfallback.json`): hierarchy quality matters more than
  layout tuning.
- **Friendster, 2026-11-25** (#15, FINDINGS § M1-01; idle; Louvain async, 1 iteration,
  f32 weights, 32 threads;
  `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl`): the plain hierarchy has a parent with 18,723,769
  children, and the lab layout was killed on it even with 56 GiB (rows 20–21). The capped
  hierarchy (cap 64, ratio 3) has fan-out ≤ 64 on four of six levels, then 1,094 and 6,478
  at the top; its Q is 0.5757 against 0.6038 plain (2.8 points; under 56 and 16 GiB; the
  other capped repetition gave 0.5771). Capped coarsening shrinks slowly (level 1:
  10.5 M communities against 1.16 M plain; capped Leiden's pass 3 still 1.42 B entries),
  and with hierarchy output the prototype keeps every level's quotient graph in memory:
  that kills the capped Leiden at 56 GiB (about 51.5 GiB of quotient graphs, arithmetic;
  not tested without hierarchy output). Under 16 GiB both capped runs were killed in
  pass-0 aggregation, while plain Louvain fit with 4.2 GiB (rows 9, 11–15). So the map
  hierarchy cannot be had by capping during coarsening on a 16 GiB machine as the
  prototype does it; shaping must be bounded in memory (#19, #63).
- Known bug in the prototype: `--cap` is ignored after the first iteration. #19 fixed it
  (FINDINGS § M1-05, "The cap baseline's known bug").
- Not tested: recursive sub-clustering, nested resolution sweeps; any graph that is not
  social.

### Consequences (2026-11-25)

- The partition is the analysis result; the map hierarchy is a derived artifact (#62,
  #63) and never reported as "the communities".
- Regions that are not communities are drawn as such on the map (`design/DECISIONS.md`,
  artefact regions hatched and labelled).

### What would change our mind (2026-11-25)

- #19 finds a method whose hierarchy has fan-out in 4–64 for most children and whose
  finest level is within one modularity point of the best partition; then one output
  could serve both and this decision is simplified.
- #19 cannot produce regions that mean something on three graphs; then the map hierarchy
  needs another source than community detection.

## Closed by

Productionized in #63.
