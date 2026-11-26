---
title: Viewer performance budgets and accessibility
milestone: M5 · The map
labels: type:infra, area:viewer, P1, risk:medium, size:M, lab-evidence
---
**Why.** Lab frame times: 16.67 ms on an RTX 4090 (vsync-capped, so no headroom figure), 29–136 ms mean and 567 ms worst with software rendering. Without budgets enforced in CI the viewer will regress on the hardware most users have.

**Scope**
- [ ] Reference machines and budgets from {{#m1-12-viewer-no-gpu}}: discrete GPU 60 fps, integrated GPU ≥ 30 fps, software rendering "usable" with reduced detail
- [ ] Automated frame-time tests with Playwright (uncapped frame timing, scripted zoom/pan) on a self-hosted runner with a GPU and in software mode
- [ ] Adaptive detail: instance and edge caps driven by measured frame time
- [ ] Bundle size budget for the viewer (target ≤ 150 KB gzip)
- [ ] Accessibility: keyboard navigation, contrast, colour-blind-safe defaults, reduced-motion setting. The screen-reader route is no longer a summary of the current view — a summary is alt text, which fails (`docs/research/sota-human-factors.md` § 2) — but the navigable hierarchy of {{#m5-16-nonvisual-navigation}}, split out of this issue on 2026-11-25 (`docs/research/README.md` H2)
- [ ] A WCAG 2.2 AA checklist in CI (decided 2026-11-25, `docs/research/README.md` H5; `docs/research/sota-human-factors.md` challenge 5), Playwright beside the frame tests: 1.4.11 non-text contrast on the dark canvas (region outlines of 1.5 px at 12%/5% tint and focus rings need 3:1; not measured yet), 2.5.7 panning without dragging (keys or buttons), 1.4.10 reflow at 320 CSS px, 4.1.3 status messages (tile loading, "shown X of Y"), 2.3.3 reduced motion, 200% browser zoom scaling the map's text; automated tools plus a manual checklist. Run it on `design/preview/style/viewer.html` first, to list the failures the build must fix
- [ ] Browser matrix: Chromium, Firefox, Safari (desktop and mobile)

**Acceptance**
- CI fails when a scripted session exceeds the frame budget on the reference configurations
- CI fails when an automated WCAG 2.2 AA check of the list above fails

**Depends on** {{#m5-05-viewer-core}}, {{#m1-12-viewer-no-gpu}} (the reference machines and budgets)

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Contrast, focus, hit-target and colour-independence requirements are stated and measured in `design/preview/style/foundations.html` (§9) and `data.html` (§1–2); the viewer must meet the same figures.
