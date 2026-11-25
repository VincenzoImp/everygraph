# ADR 0016 — Two community outputs: the partition and the map hierarchy

- **Status:** accepted, partly validated
- **Decision:** D16 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Users want the best partition (for analysis and papers). The map wants zoom levels with a
bounded number of children per region (for layout and tiles). One hierarchy does not
serve both.

## Decision

Community detection produces two outputs: the partition with the best modularity, and a
map hierarchy shaped for the map (fan-out capped, levels compacted).

## Evidence

Idle, 16 threads, async Leiden unless noted (FINDINGS § B2 "Hierarchy for the map";
`lab/b2-leiden/results/hier/`, "Hierarchies (whole machine)" in
`lab/b2-leiden/results/summary.txt`):
- **Natural hierarchies are unusable for a map**: the largest parent has 102,821 children
  on lj and 338,769 on orkut (level 1, compaction ratio 3). DESIGN and FINDINGS say
  "~100k"; the idle orkut run is 3.4× that. Louvain is shallower still (orkut 3.07 M →
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
- Known bug in the prototype: `--cap` is ignored after the first iteration (#19).
- Not tested: recursive sub-clustering, nested resolution sweeps; any graph that is not
  social.

## Consequences

- The partition is the analysis result; the map hierarchy is a derived artifact (#62,
  #63) and never reported as "the communities".
- Regions that are not communities are drawn as such on the map (`design/DECISIONS.md`,
  artefact regions hatched and labelled).

## What would change our mind

- #19 finds a method whose hierarchy has fan-out in 4–64 for most children and whose
  finest level is within one modularity point of the best partition; then one output
  could serve both and this decision is simplified.
- #19 cannot produce regions that mean something on three graphs; then the map hierarchy
  needs another source than community detection.

## Closed by

#19 (hierarchy shaping); productionized in #63.
