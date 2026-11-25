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
- [ ] Preflight and resume of ingest as `docs/ux-contract.md` § 3 and § 5: scratch in `.everygraph-ID` beside the output, memory-backed scratch counted against the budget (P-5, P-18); the fingerprint of R-4, with input identity, phase versions and budget-dependent layout checked against the manifest (R-3, R-7); the `old-scratch` warning (R-10); and the tests § 8 assigns to this issue (interruption at each phase with a resume under another `--threads` and a smaller `--memory`, `kill -9`, fingerprint and R-7 table tests, clean-up tests)

**Acceptance**
- Estimates within ±30% of actuals on the tier-2 datasets, for wall time, peak memory, scratch and output size, measured by the benchmark harness on an idle machine (`docs/ux-contract.md` P-7, P-9); no test in the suite asserts a wall time
- `kill -9` at ten random points during a Friendster ingest: every resume completes and the result is byte-identical to an uninterrupted run
