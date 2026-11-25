---
title: Ratify design decisions D1–D25 as ADRs
milestone: M0 · Foundations and decisions
labels: type:decision, area:foundations, P0, risk:low, size:M, lab-evidence
---
**Why.** `docs/DESIGN.md` lists 25 decisions with their evidence. Before code depends on them, each needs an explicit accept / revise / needs-experiment verdict, so later changes are deliberate.

**Scope**
- [ ] One ADR per decision under `docs/adr/` (context, decision, evidence with link to FINDINGS, consequences, what would change our mind)
- [ ] Mark the partly validated ones and link the experiment that closes them: D7 `.eg` container (unvalidated) → {{#m2-01-container-spec}}; D11 for community detection → {{#m1-02-ooc-aggregation}}; D14 cost → {{#m1-04-det-leiden-scaling}}; D16 → {{#m1-05-hierarchy-shaping}}; D17 → {{#m1-07-layout-area}}
- [ ] The ADR for D14 records its widening of 2026-11-25 (`design/DECISIONS.md`, "What 'repeatable' promises"): identical results also across memory budgets and execution regimes, with the tests that hold it ({{#m3-11-determinism-ci}}) and the case that would narrow it (out-of-core Leiden, {{#m4-04-ooc-leiden}}); `docs/DESIGN.md` D14 is updated in the same pull request
- [ ] Record the three initial assumptions the lab overturned (FST dictionary, raw-by-default storage, "determinism within 30%") so they are not reintroduced

**Acceptance**
- 25 ADRs merged; each open one names its owner experiment
- DESIGN.md decision table links to the ADRs
