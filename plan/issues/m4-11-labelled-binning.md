---
title: Map hierarchy: labelled binning for leaves that cannot be split
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:low, size:S, lab-evidence
---
**Why.** Some oversized leaves of the map hierarchy cannot be split: re-clustering returns one community, typically a star. On Friendster, `rec` leaves 8 such leaves, and the lab layout invents 99 unlabelled groups inside them (PR #162). These are the last artificial regions that the map does not label as such, against an acceptance criterion of {{#m1-05-hierarchy-shaping}} and {{#m4-07-map-hierarchy}}.

ASK-GraphView (2006) has a fallback for exactly this case. When the clusterer returns one cluster, an overfull node's children are binned under new nodes of bounded fan-out by a contraction-based construction (`docs/research/sota-communities.md` § 3 D16, § 4 challenge 5).

**Scope**
- [ ] When re-clustering returns one community, split the leaf by a balanced, deterministic method into tree-level regions of the "subdivision" kind (drawn dashed, as approved: `design/preview/style/SPEC.md` "The map"). Candidates: ASK-GraphView's contraction-based binning; a Paris dendrogram cut with a size bound (scikit-network's `cut_balanced`); a seeded balanced bisection. These leaves hold at most about 900 nodes on Friendster, so the work is in RAM
- [ ] The layout ({{#m5-01-layout}}) draws these regions instead of deriving its own groups
- [ ] Budgeted and identical at any thread count

**Acceptance**
- On Friendster and lj, the count of layout-derived groups drops to 0 (Friendster: 99 today) and every drawn region carries its kind
- The hierarchy hash is identical at 1, 4 and 16 threads

**Pointers.** PR #162, "Prior art and follow-ups"; `docs/research/sota-communities.md` § 4 challenge 5; `docs/research/sota-maps.md` § 4 challenge 7.

**Depends on** {{#m1-05-hierarchy-shaping}}
