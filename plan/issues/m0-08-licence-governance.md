---
title: Licence, contribution rules and governance
milestone: M0 · Foundations and decisions
labels: type:decision, area:foundations, P1, risk:low, size:S
---
**Why.** Recent history in this space (Kùzu acqui-hired and archived, Memgraph MAGE relicensed, single-maintainer projects going dormant) makes governance a selection criterion for adopters.

**Scope**
- [ ] Dual licence Apache-2.0 OR MIT (DESIGN D23); `cargo deny` allow-list that keeps the dependency tree permissive
- [ ] `CONTRIBUTING.md`, DCO sign-off, code of conduct, security policy
- [ ] Commitment in the README: format specifications are public and versioned; no feature gates behind a paid edition
- [ ] Decide what stays out of the permissive core, if anything (e.g. optional webgraph feature, licences of bundled fonts/JS)
- [ ] Bus-factor plan: second maintainer with release rights before v0.1; note on a foundation path (see {{#m9-08-sustainability}})

**Acceptance**
- Licence files and policies merged; dependency licence check green in CI
