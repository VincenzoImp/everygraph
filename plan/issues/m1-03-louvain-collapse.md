---
title: Diagnose the async Louvain quality collapse on Friendster
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:community, P0, risk:high, size:M, lab-evidence
---
**Question.** Why does asynchronous Louvain return Q 0.4647 on Friendster (35,682 communities) when Leiden returns 0.6033, with passes 1–4 each hitting the 20-iteration cap with millions of moves?

**Not reproduced (tracer, {{#m1-01-e2e-tracer}}, 2026-11-25, idle; `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 15; `lab/m1-01-tracer/results/b2_louvain_plain_16G.json` against `lab/b2-leiden/results/big/friendster_louvain_semiext16g.json`).** Plain async Louvain under `MemoryMax=16G` (1 iteration, f32 weights, `--lowmem`, 32 threads, as before; this time idle and with `--min-ratio 3`) gave **Q 0.6038**, 33,306 communities, the largest 19.8 M nodes, against Q 0.4647, 35,682 and 33.4 M in the shared-machine run. Its passes 1–3 converged in 6, 3 and 7 iterations (41 in total; pass 4 hit the cap), where the earlier run's passes 1–4 each hit the 20-iteration cap (85 in total). One run each; cause not diagnosed: async races, `--min-ratio`, or machine state. The two capped Louvain repetitions at 56 GiB also differ (Q 0.5771 with 80 top communities, 0.5757 with 52; rows 13–14).

**Why it matters.** Until explained, it is unknown whether the cause also affects Leiden at larger scales, the deterministic mode, or other graph families — and whether async mode should ship at all.

**Hypotheses to test**
- [ ] Herding on stale community totals in pass 0 at 32 threads (compare 1, 4, 16, 32 threads; compare deterministic mode, which uses frozen decisions)
- [ ] Oscillation on coarse levels (count vertices that move back within two iterations)
- [ ] f32 quotient weights losing exactness above 16.7 M (the Friendster run used the `w32` build; orkut already exceeds that bound at the top levels) — rerun with f64
- [ ] Interaction between the iteration cap, tolerance schedule and graph size
- [ ] Page-cache thrash changing the effective visiting order under the 16 GiB limit (rerun unlimited)
- [ ] First, reproduce: the FINDINGS § B2 configuration and the tracer's configuration back to back on the idle machine, at least 5 repetitions each, under 16 GiB and unlimited, to separate run-to-run variance from `--min-ratio` and from machine state. If Q ≥ 0.60 every time, record the collapse as a shared-machine artefact and narrow the issue to the thread-count drift below

**Done when.** The cause is identified with a minimal reproduction, a fix or a guard is proposed, and the async quality drift with thread count seen on orkut (Leiden 0.6836 at 1 thread → 0.6743 at 32) is explained or bounded.

**Feeds.** {{#m4-01-leiden-production}}; decision on whether a non-deterministic mode ships.
