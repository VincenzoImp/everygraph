---
title: Harness: randomized interleaved rounds for A/B runs and suites
milestone: M3 · Engine and kernels
labels: type:infra, area:bench, P1, risk:low, size:S
---
**Why.** CLAUDE.md § 6 requires decisions between two variants to come from "interleaved A/B runs in the same process or the same harness run", but the harness runs cases in spec order with the repetitions of a case back to back, so it cannot interleave. Consecutive trials "falsely report" 37.8% between identical systems (Abedi–Brecht, ICPE'17, EC2 traces); on 1,700 bare-metal servers fixed and random order differ by 50% or more, and 72% of configurations could change a conclusion (Duplyakin et al., ATC'23); same-instance randomized trials detect slowdowns ≤ 10% (Laaber et al.). ADR 0026 itself was an ordering effect. Interleaving is also what makes A/B decisions robust to background load on this shared host. Decided 2026-11-25 (`docs/research/README.md` M3).

**Scope**
- [x] `run --interleave --seed S`: rounds over the selected cases, or over two binaries or specs for an A/B; the order reshuffled each round (seeded), recorded per row as `round` and `position`; warm-ups stay per case
- [x] Opt-in per spec where the drive matters: interleaving writers with readers changes drive state ({{#m1-21-drive-state}}), and cold-cache rotation needs care
- [x] `compare` accepts the rounds of one interleaved run and gives the ratio interval of {{#m3-18-harness-statistics}}
- [ ] First use: `kernels-orkut` and `ingest-text` interleaved and consecutive in the same idle window, n = 10 each; medians and interval widths compared

**Acceptance**
- An A/B of two binaries runs as one harness command and reports a ratio interval
- The comparison of interleaved against consecutive runs above is recorded with its result file

**Status (2026-11-26).** Built: `run --interleave --seed S` (seeded random rounds, each even round the reverse of the one before), `round`, `position` and `seed` in every row, the spec key `interleave`, `report --compare --by VAR=VALUE` and `run --compare-by VAR=VALUE` (test `an_interleaved_ab_of_two_binaries_reports_a_ratio_interval`). The first use, interleaved against consecutive on an idle machine, waits for the M3 official window.

**Feeds.** Every A/B the plan asks for (e.g. {{#m1-08-memory-bandwidth}}, {{#m4-03-parallel-aggregation}}, {{#m2-28-par-scatter}}), {{#m8-03-public-benchmarks}}.

**Pointers.** `docs/research/sota-measurement.md` § 2 ("In which order?"), § 3.1 (Order row), § 4 challenge 3; `crates/everygraph-bench/src/harness/run.rs`.

**Depends on** {{#m0-03-bench-harness}}
