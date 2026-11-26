---
title: M1 epic: de-risking lab, round 2
milestone: M1 · De-risking lab, round 2
labels: type:epic, area:bench, P0
---
Tracking issue for milestone M1.

**Goal.** Turn every item in DESIGN § 5 ("What is not solved yet") into a number and a decision before production code depends on it. Each experiment is time-boxed, uses the lab prototypes in `lab/`, runs through the benchmark harness, and ends with an ADR update. A decision rests on load-independent results and interleaved A/B ratios; idle-machine runs are needed only for numbers that go into documents, and they run in the milestone window (CLAUDE.md § 6, 2026-11-26). A clear negative result is a valid outcome.

**Ordered by risk to the product**
1. {{#m1-01-e2e-tracer}} — does the whole pipeline hold together on 1.8 B edges under 16 GiB?
1. {{#m1-13-compressed-end-to-end}} — what do the headline numbers become on the default (compressed) storage?
2. {{#m1-02-ooc-aggregation}} — out-of-core Leiden
3. {{#m1-03-louvain-collapse}} — unexplained quality collapse at scale
4. {{#m1-05-hierarchy-shaping}} — hierarchy the map can use
5. {{#m1-06-edge-rendering}} — readable edges
6. {{#m1-04-det-leiden-scaling}} — price of determinism
7. {{#m1-07-layout-area}}, {{#m1-08-memory-bandwidth}}, {{#m1-09-hierarchy-order}}
8. {{#m1-10-graph-families}}, {{#m1-11-real-inputs}}, {{#m1-12-viewer-no-gpu}}
9. From the first literature review (2026-11-25, `docs/research/README.md`): {{#m1-14-relabel-at-ingest}}, {{#m1-15-mmap-hints-block-cache}}, {{#m1-16-codec-0124}}, {{#m1-17-chunked-hub-records}}, {{#m1-18-layout-baseline}}, {{#m1-19-networkx-dispatch}}, {{#m1-20-ldbc-graphalytics}}
10. From the second literature review (2026-11-25, `docs/research/README.md` D): {{#m1-21-drive-state}}

**Exit criteria**
- [ ] Each experiment closed with a result file, a FINDINGS section and an ADR verdict
- [ ] DESIGN § 6 success criteria confirmed reachable, or rewritten with honest targets
