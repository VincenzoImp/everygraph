---
title: Tracer bullet: 1.8 B edges to a browsable map under 16 GiB
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:engine, P0, risk:high, size:L, lab-evidence
---
**Question.** The lab measured each stage separately. Does the chain text → CSR → communities → hierarchy → layout → tiles → viewer complete on com-Friendster inside a hard 16 GiB limit, and what are the per-stage time, peak memory and disk?

**Why first.** This is the product's headline claim and it has never been run end to end. Layout and tiles on Friendster are extrapolations (2–4 min, ~20 GB RAM, 9–13 GB of tiles); community detection under 16 GiB currently either fails (Leiden needs 23.6 GB anonymous) or returns a poor partition (Louvain, Q 0.465).

**Method**
- [ ] Glue the lab binaries (`lab/b1-core` ingest, `lab/b2-leiden`, `lab/e-map` layout + tiles + viewer) with a driver script; no new algorithms
- [ ] Run each stage in its own `MemoryMax=16G, MemorySwapMax=0` scope; record wall time, cgroup peak, anonymous peak, temp disk, output size
- [ ] Repeat unlimited on the same machine to get the slowdown per stage
- [ ] Open the resulting map in the viewer; capture first-view bytes, session bytes and screenshots at five zoom levels
- [ ] List every stage that fails or needs > 16 GiB, with the reason

**Decision it feeds.** Which stages block the laptop claim, and the realistic wording of DESIGN § 6.

**Done when.** A table "stage × {time, peak memory, disk, status}" for limited and unlimited runs is in FINDINGS, with screenshots; follow-up issues exist for each failing stage.

**Pointers.** FINDINGS § B1, § B2 (Friendster rows), § E (extrapolation); tile builder holds all tiles in RAM (known risk).
