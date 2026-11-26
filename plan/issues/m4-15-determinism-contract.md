---
title: Determinism contract stated exactly, and its cheap holes closed
milestone: M4 · Communities and hierarchy
labels: type:spec, area:community, P0, risk:medium, size:S
---
**Why.** D14 promises bit-identical results for a view as a mask or materialized. The state-of-the-art review of 2026-11-26 (`docs/research/sota-review-2026-11.md` § 1 rows 5–8; `communities.md` § 0.1) found four places where the code keeps less than that:
- Leiden's sub-round count is `clamp(next_pow2(n / 65,536), 4, 1,024)`, with `n = plan.nodes()`, and its draws and tie order use stored ids (`crates/everygraph-community/src/level.rs` l. 146–149, 303–320). A materialized view has another n and other ids.
- `compact --order` renumbers ids, so the communities change.
- The super level's hysteresis makes a map depend on the previous map (ADR 0016).
- Deterministic Leiden is about 1 modularity point below async on orkut (`lab/m1-04-det-leiden-scaling/README.md` finding 7), and D14 does not say so.

**Scope**
- [ ] ADR 0014 gains a table of the reproducibility properties, each marked guaranteed (with its test), shown in the lab, or not promised: same seed; same thread count; any thread count; scheduling order; in-core vs out-of-core; across CPUs; CPU vs GPU; bitwise; partition up to renumbering; same input order vs a permuted one; view as mask vs materialized.
- [ ] Views: a masked and a materialized view give the same communities. Either the sub-round rule and the draws depend on the view's own nodes in view order, or D14 drops the claim. Decide from the determinism suite's test (#54).
- [ ] `compact --order`: D14 says whether "same input" includes the stored order. If it does not, Leiden runs in a canonical order.
- [ ] The hysteresis: state it in D14, or remove it.
- [ ] The quality cost of the deterministic mode, measured on production Leiden over several seeds (modularity does not depend on load), is written beside the promise.

**Done when.** Every row of the table has its test or its "not promised", and #54's tests for views and `compact --order` pass or are removed with the claim.

**Depends on** {{#m3-11-determinism-ci}}, {{#m4-01-leiden-production}}
