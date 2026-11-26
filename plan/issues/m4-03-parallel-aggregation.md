---
title: Aggregation rewrite: the scaling bottleneck
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:medium, size:M, lab-evidence
---
**Merged into the M4 performance pass (2026-11-26).** Each part of this issue now has another owner, and it is closed:
- One aggregation in every regime, the sequential bucket aggregation, is built by {{#m4-01-leiden-production}}. The choice between hash and bucket aggregation per level is dropped.
- Quotient weights as i64 fixed point: {{#m4-02-weights-resolution}}.
- Coarse levels written in the standard adjacency format: {{#m4-06-hierarchy-artifact}} and {{#m4-04-ooc-leiden}}.
- No `unsafe` scatter: done in {{#m4-01-leiden-production}} with the helpers of {{#m2-28-par-scatter}}.
- The two speed targets go to the M4 performance pass (`m4-12-performance-pass`): aggregation ≤ 25% of total time at 16 threads on lj and orkut; Friendster in-RAM Leiden, one async iteration, 16 threads, at 0.71× the baseline re-measured in the same configuration (82.4 s idle, #118, `lab/m1-13-compressed/results/leiden_big.jsonl`). First measured there: parallel prefix sums, buffer reuse across passes, and the two-pass path that avoids the O(m) temporary (lab: lj peak 1.65 → 1.00 GB).

**Depends on** {{#m4-01-leiden-production}}
