---
title: Label propagation as an instant preview
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P2, risk:low, size:S
---
**Why.** On very large graphs, or for a first look, a partition in a couple of scans is useful — for example to produce a quick preview map while Leiden runs, or as a locality ordering. It is a preview only, never a low-budget fallback: our out-of-core Leiden already runs Friendster under 4 GiB (3.49× the in-RAM time, Q about 0.60, FINDINGS § M1-02).

**Scope**
- [ ] Deterministic synchronous label propagation with **LDBC Graphalytics CDLP semantics** (decided 2026-11-25, `design/DECISIONS.md`, "Label propagation follows LDBC CDLP"): the initial label of a node is its position in the original key order, ties go to the smallest label, a fixed number of iterations; on the edge-map framework, semi-external capable. Not the earlier tie-break by label hash, which differs from the only standard deterministic LPA
- [ ] If the stored ids were relabelled (#146), labels and ties are still decided on the original key order
- [ ] Clearly labelled as a preview in output provenance; not a substitute for Leiden

**Not adopted (2026-11-25): CluStRE as a quick preview or low-budget fallback** (`docs/research/sota-communities.md` § 3, challenge 10). Corrected numbers: on Friendster, Q 0.5871 is CluStRE's Strong mode (memetic, 15 s evolutionary time limit); Light+ gives 0.5839, Light 0.5229 (CluStRE Table 1, single core, their Xeon Silver 4216). Its local search stops on a 10-minute wall-clock cut-off, so it is not deterministic for a fixed stream order. It sits below what our out-of-core Leiden already gives.

**Acceptance**
- Friendster preview partition in < 30 s under a 4 GiB budget
- Matches the LDBC Graphalytics CDLP reference outputs exactly on the LDBC test graphs and on com-friendster (the reference outputs from {{#m1-20-ldbc-graphalytics}})
