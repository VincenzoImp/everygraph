---
title: Preflight, progress and resume: the UX contract
milestone: M0 · Foundations and decisions
labels: type:spec, area:foundations, P1, risk:low, size:S
---
**Why.** "Never run out of memory" and "works at any size" are only credible if the tool tells the user what it is about to do and survives interruption. The old largegraph prototype already had resumable phases and a `--dry-run`; that discipline carries over.

**Scope**
- [ ] Preflight output format: regime chosen and why, estimated wall time, peak RAM, temp disk, output size; refusal message when the O(n) state cannot fit the budget, with the smallest budget that would work
- [ ] Progress reporting contract (phases, units, ETA) shared by CLI, Python and the Jupyter widget
- [ ] Resume semantics: which operations checkpoint, what is reused after a crash, how stale partial files are detected
- [ ] Cancellation semantics (Ctrl-C, Python `KeyboardInterrupt`) without corrupting the `.eg` file

**Acceptance**
- Spec merged; referenced by {{#m2-13-preflight-resume}} and {{#m3-02-planner}}
