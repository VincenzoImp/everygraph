---
title: Temporal segments and incremental appends
milestone: M9 · After v0.1
labels: type:feature, area:format, P1, risk:high, size:XL, lab-evidence
---
**Why.** v0.1 graphs are immutable snapshots. Real datasets grow daily (new blocks, new messages). The design from {{#m7-05-temporal}}: append-only, time-partitioned adjacency segments, LSM-style — "as of T" is a union of whole segments with only boundary segments filtered, and appending is writing a new segment.

**Scope**
- [ ] Segment format and manifest inside `.eg`; merge/compaction policy. Already fixed by the 0.1 container and kept: required feature bit 0 `segments`, set exactly when the file has segments; the manifest as kind 6, named `segments`; segment sections named `seg/NNNNNNNN/…`; the graph identity of a segmented file (`docs/spec/eg-container.md` §§ 4, 6, 10, 13). This issue specifies the manifest's byte layout, the merge policy and the reading rules
- [ ] Id dictionary growth: layered, new keys get new dense ids after the existing ones, and a merge that rebuilds the MPHF keeps every id (ADR 0028, S5)
- [ ] Kernels over multi-segment sources through the edge-map framework
- [ ] Incremental maintenance where cheap (degrees, components by union-find); recompute otherwise. Stored results are keyed by the graph identity, which for a segmented file covers the manifest and so the ordered list of runs (`docs/spec/eg-container.md` § 6)
- [ ] Incremental community update and map stability across updates (research: dynamic Leiden variants)
- [ ] Time-respecting path queries
- [ ] The rules already written for segments (ADR 0028): `.events` per run; the reserved `sparse` index flag (`docs/spec/eg-adjacency.md` § 3); byte-identical merges (§ 4); pair aggregates stored by the newest run that touches a pair, over all runs, superseding older values; `duplicates="sum"` with float weights, and an append that would overflow a pair sum's type, not appendable (`docs/spec/graph-semantics.md` § 4.3); statistics per run and updated exactly and incrementally for the whole graph (§ 2.7); graph order and a layered dictionary that keeps every id on merge (§ 2.3, ADR 0008); results that do not depend on how runs are merged or compacted (`design/DECISIONS.md`)

**Acceptance**
- Appending 1% new edges to a 1 B-edge graph takes time proportional to the new data, and "as of" queries at 100 time points do not rescan the whole graph

**Second research wave (2026-11-25, `docs/research/README.md` D1, D2, D4).** No code here before {{#m9-11-compaction-invariant}} has fixed how results stay independent of merging.
- *Storage:* a levelled log of compressed, immutable runs, each in the D4 format with the sparse segment index; a levelled (Bentley–Saxe) merge with each level about 10× the one above (LSMGraph's T = 10, ≤ 5 levels, MemGraph 64 MB; SIGMOD'24, their machine); a merge is a pure function of its inputs; the manifest holds event-time ranges, and "as of T" takes whole runs with `t_max ≤ T` and filters runs with `t_min ≤ T < t_max`. Append cost is then proportional to Δ (amortized O(log_T(m/Δ)) rewrites per event, `docs/research/sota-dynamic.md` C4). That the edge-map sources must accept a merging source was sent to #44's author. Experiment before building: lj and orkut with a fixed random time stamp per edge, base 90% + 10 runs of 1%, PageRank iteration and BFS on 1, 2, 4 and 11 runs against compacted, 16 threads, idle
- *Later work, research:* incremental Leiden and stable map updates. Both can be deterministic: synchronous rounds with a total-order tie-break (LD-Leiden is one step away; it still varies 0.38% in Q on average across 1–64 threads, its machine) and exact repair of the per-level aggregates of {{#m4-06-hierarchy-artifact}} (integer weights). But their results depend on the batch sequence: no incremental method returns the from-scratch partition (memberships matched 84% down to 44% after delete-and-reinsert in Sahu's heuristic dynamic Leiden; delta-screening gave Q 0.658227 against 0.652412 by the order within one batch, their machines). So D14's "same input" means the same base and the same sequence of batches, as the `design/DECISIONS.md` row on files built by appends says, and incrementally maintained results are cached keyed by the ordered batch list. For D17, a stable map update is a function of the new graph and the previous map file, not of the graph alone: anchored layout, regions matched to the previous map by maximum-weight overlap with a fixed tie-break, hysteresis on moves, a full relayout of a level when its quality falls a fixed margin below a fresh one (`docs/research/sota-dynamic.md` C6, C7)
- *Retractions:* deletions as retraction runs get only a reserved run flag in the manifest (under the `segments` required bit), written into the manifest's layout by this issue; they are not built (C8)

**Depends on** {{#m7-05-temporal}}, {{#m9-11-compaction-invariant}}
