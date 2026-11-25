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
- [ ] Accessibility: keyboard navigation, screen-reader summary of the current view, contrast, colour-blind-safe defaults, reduced-motion setting
- [ ] Browser matrix: Chromium, Firefox, Safari (desktop and mobile)

**Acceptance**
- CI fails when a scripted session exceeds the frame budget on the reference configurations

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Contrast, focus, hit-target and colour-independence requirements are stated and measured in `design/preview/style/``foundations.html` (§9) and `data.html` (§1–2); the viewer must meet the same figures.
