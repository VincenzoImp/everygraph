---
title: Diagnose the async Louvain quality collapse on Friendster
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P0, risk:high, size:M, lab-evidence
---
**Question.** Why does asynchronous Louvain return Q 0.4647 on Friendster (35,682 communities) when Leiden returns 0.6033, with passes 1–4 each hitting the 20-iteration cap with millions of moves?

**Why it matters.** Until explained, it is unknown whether the cause also affects Leiden at larger scales, the deterministic mode, or other graph families — and whether async mode should ship at all.

**Hypotheses to test**
- [ ] Herding on stale community totals in pass 0 at 32 threads (compare 1, 4, 16, 32 threads; compare deterministic mode, which uses frozen decisions)
- [ ] Oscillation on coarse levels (count vertices that move back within two iterations)
- [ ] f32 quotient weights losing exactness above 16.7 M (the Friendster run used the `w32` build; orkut already exceeds that bound at the top levels) — rerun with f64
- [ ] Interaction between the iteration cap, tolerance schedule and graph size
- [ ] Page-cache thrash changing the effective visiting order under the 16 GiB limit (rerun unlimited)

**Done when.** The cause is identified with a minimal reproduction, a fix or a guard is proposed, and the async quality drift with thread count seen on orkut (Leiden 0.6836 at 1 thread → 0.6743 at 32) is explained or bounded.

**Feeds.** {{#m4-01-leiden-production}}; decision on whether a non-deterministic mode ships.
