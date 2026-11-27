---
title: README and repository page: a first pass during development
milestone: M3 · Engine and kernels
labels: type:docs, area:docs, P1, risk:low, size:S
---
**Why.** The repository is public during development (`design/DECISIONS.md`, "When the repository becomes public"), and its page is the project's first impression. The README had grown to a long table of lab rows marked † and ‡, a status line that no longer said what works, and no way to try the software. The release README is {{#m8-08-release}} (a demo, a quickstart, a comparison table) and the brand files are {{#m8-14-brand-files}}; this is the interim pass, built to the README header of `design/preview/style/web.html`.

**Scope**
- [ ] README: a header with the Nest mark that follows GitHub's light and dark themes (`<picture>`, `prefers-color-scheme`, `design/system/mark/`); badges for CI on `main`, the licence, the MSRV and the status; a one-paragraph pitch, the three promises, what works today, what is not solved yet (linking `docs/DESIGN.md` § 5), a quickstart a reader can run today, where to look, contributing, the licence and no telemetry; about half the previous length
- [ ] No performance number in the README; the official product results move to a page linked from it, `bench/results/README.md`, marked as development builds on one machine, each table linked to its result file
- [ ] The Markdown files the README links to first: no broken links, no stale statement, a first screen that says what the page is
- [ ] A social-preview image, 1280 × 640, generated from the design system by a script (`design/system/social/`)
- [ ] Proposed repository description (at most 350 characters), 10–20 topics and the homepage field, for the coordinator to apply; the owner uploads the social preview in Settings (GitHub's API cannot set it)
- [ ] Departures from the approved README header and social card recorded in `design/DECISIONS.md`

**Acceptance**
- The quickstart's commands run as written on a clean checkout, under `systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0`
- Every number on the README and on the results page traces to a file under `bench/results/`
- The README renders correctly in GitHub's light and dark themes (`gh api markdown`, screenshots in the pull request)
- `docs/tools/check_adrs.py`, `docs/tools/check_api_sketch.py`, `design/system/tools/lint_colours.py` and `plan/publish.py --dry-run` pass

**Depends on** nothing.

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** `design/preview/style/web.html`, "README header on GitHub" and "Social card and slide"; `plan/issues/m8-02-docs-site.md`: no performance numbers on the landing until the released software is measured.
