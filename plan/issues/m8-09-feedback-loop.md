---
title: Feedback loop without telemetry
milestone: M8 · CLI, docs and v0.1 launch
labels: type:infra, area:launch, P2, risk:low, size:S
---
**Why.** The north-star metric is "people who reached their first map". A local-first tool should not phone home, so adoption has to be read from public signals and direct contact.

**Scope**
- [ ] No telemetry in the engine or viewer; say so in the README
- [ ] Issue templates (bug with `everygraph doctor` output, performance report with `everygraph-bench` spec, dataset that broke something), discussions enabled
- [ ] Public signals tracked monthly: downloads, stars, public `.egmap` files found in the wild, citations, questions asked
- [ ] A "send us your worst graph" call: a structure-only export (`everygraph export --topology-only`) so users can share a failing case without its properties. Never called "anonymized" (changed 2026-11-25, `docs/research/README.md` H7): structure alone can identify people — Narayanan and Shmatikov re-identified 30.8% of the mapped users of an anonymized Twitter graph (S&P'09). The command prints a one-line notice saying so, and the call asks for the data owner's consent
- [ ] Office hours / a chat channel for early adopters

**Acceptance**
- Templates live; first monthly report written

**Depends on** {{#m8-01-cli}}
