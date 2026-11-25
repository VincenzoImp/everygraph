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

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Wording, layout and tone of plans, progress, refusals and warnings are shown in `design/preview/cli.md` and typeset in `design/preview/style/``terminal.html` and `notebook.html`: summary of what was understood, plan with estimates, one line per phase, a result line and a suggested next command; never asks before starting (`--dry-run` plans only); default memory budget is half of free RAM; errors state what, why with the arithmetic, and the ways out as copyable commands; output fits 80 columns and degrades to no-colour and ASCII.
