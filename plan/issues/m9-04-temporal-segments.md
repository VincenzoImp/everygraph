---
title: Temporal segments and incremental appends
milestone: M9 · After v0.1
labels: type:feature, area:format, P1, risk:high, size:XL, lab-evidence
---
**Why.** v0.1 graphs are immutable snapshots. Real datasets grow daily (new blocks, new messages). The design from {{#m7-05-temporal}}: append-only, time-partitioned adjacency segments, LSM-style — "as of T" is a union of whole segments with only boundary segments filtered, and appending is writing a new segment.

**Scope**
- [ ] Segment format and manifest inside `.eg`; merge/compaction policy
- [ ] Id dictionary growth (new keys get new dense ids; MPHF rebuilt or layered)
- [ ] Kernels over multi-segment sources through the edge-map framework
- [ ] Incremental maintenance where cheap (degrees, components by union-find); recompute otherwise, with the artifact cache keyed by segment set
- [ ] Incremental community update and map stability across updates (research: dynamic Leiden variants)
- [ ] Time-respecting path queries

- [ ] The rules already written for segments (ADR 0028): `.events` per run; the reserved `sparse` index flag (`docs/spec/eg-adjacency.md` § 3); byte-identical merges (§ 4); pair sums exact across runs and `duplicates="sum"` with float weights not appendable (`docs/spec/graph-semantics.md` § 4.3); statistics per run and recomputed for the whole graph (§ 2.7); graph order and a layered dictionary (§ 2.3, ADR 0008); results that depend on the batches only (`design/DECISIONS.md`)

**Acceptance**
- Appending 1% new edges to a 1 B-edge graph takes time proportional to the new data, and "as of" queries at 100 time points do not rescan the whole graph

**Depends on** {{#m7-05-temporal}}
