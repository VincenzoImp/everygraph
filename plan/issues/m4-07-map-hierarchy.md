---
title: Map hierarchy builder
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P0, risk:high, size:L, lab-evidence
---
**Why.** The partition with the best modularity is not the hierarchy a map needs (DESIGN D16). The lab's layout experiment concluded that hierarchy quality matters more than layout tuning: a weaker hierarchy made the layout 3× worse.

**Scope.** Productionize the method selected in {{#m1-05-hierarchy-shaping}}:
- [ ] Input: graph + community partition; output: a tree with bounded fan-out stored as a hierarchy artifact ({{#m4-06-hierarchy-artifact}})
- [ ] Every tree node is typed: `community` (found by the algorithm), `subcommunity` (recursive split), `aggregate` (e.g. "1,843 small communities"), so the viewer never presents an artifact as a community
- [ ] Target fan-out configurable (default range 8–64); report the achieved distribution
- [ ] Deterministic; exact aggregates preserved through compaction (skipped levels are composed)
- [ ] The uncapped partition remains available as the analysis result
- [ ] Memory bounded by the accountant: shaping holds at most one level's quotient graph or one oversized community at a time. On Friendster the cap applied during coarsening shrinks levels slowly (level 1: 10.5 M communities against 1.16 M uncapped) and, with every level kept, the capped Leiden was killed at 56 GiB, while the plain solve needs 4.2 GiB ({{#m1-01-e2e-tracer}}, `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 10, 13–15)

**Acceptance**
- On lj, orkut and one non-social graph: ≥ 80% of internal tree nodes have fan-out in 4–64, no untyped derived regions, and top-level modularity loss vs the uncapped partition is reported
- Layout quality metric (edge length ÷ random-pair distance) no worse than with the lab's capped hierarchy
- Friendster: the map hierarchy builds under `MemoryMax=16G`, and no parent has a fan-out the layout cannot place (the plain Louvain hierarchy, fan-out up to 18.7 M, killed the lab layout even at 56 GiB; `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 20–21)

**Depends on** {{#m1-05-hierarchy-shaping}}, {{#m4-06-hierarchy-artifact}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** The three kinds of region and how each is drawn are fixed in `design/preview/style/``SPEC.md` ("The map") and shown in `data.html` §4: found community (solid outline), subdivision made for readability (dashed), many small communities shown as one (hatched). The hierarchy artifact must carry the kind of every tree node so the viewer can draw it honestly.
