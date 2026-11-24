---
title: Ingest preflight estimate and resumable builds
milestone: M2 · Format and ingest
labels: type:feature, area:ingest, P1, risk:medium, size:M
---
**Why.** A 30 GB ingest that fails at minute four for lack of temp disk, or cannot resume after a reboot, breaks the "works at any size" promise ({{#m0-12-ux-contract}}).

**Scope**
- [ ] Preflight from a sample of the input: estimated n, m, key type, peak RAM, temp disk, output size, wall time; check free disk before starting
- [ ] Checkpoints at phase boundaries (dictionary built, buckets written, sections committed) with a manifest; resume skips completed phases after validating fingerprints
- [ ] Cleanup of temp files on success, failure and cancellation
- [ ] Estimates are compared with actuals in the ingest report so the model can be tuned

**Acceptance**
- Estimates within ±30% of actuals on the tier-2 datasets
- `kill -9` at ten random points during a Friendster ingest: every resume completes and the result is byte-identical to an uninterrupted run
