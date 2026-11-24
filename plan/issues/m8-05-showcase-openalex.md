---
title: Showcase map 2: the OpenAlex citation graph
milestone: M8 · CLI, docs and v0.1 launch
labels: type:feature, area:launch, P1, risk:medium, size:L
---
**Why.** "A map of science" at full scale — roughly 250 M works and on the order of 2 B citations in OpenAlex — is a billion-edge demonstration on open data (CC0) that no free tool renders today, and it speaks to the scientometrics community, which lacks an open tool at that scale.

**Scope**
- [ ] Ingest from the OpenAlex snapshot (work ids, referenced works, year, field) — directed, with year as a temporal property
- [ ] Communities on the symmetrized graph; region labels from fields/concepts; colour by field; time slider by publication year if {{#m5-13-attribute-overlays}} is ready
- [ ] Build under a 16 GiB budget and report every stage — this is the public version of {{#m1-01-e2e-tracer}}
- [ ] Static hosting of a multi-GB map file with range requests; cost estimate for bandwidth
- [ ] Write-up with method, limits and how to reproduce

**Acceptance**
- Published map with search by DOI/title; build log showing no stage exceeded the budget
