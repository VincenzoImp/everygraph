---
title: Viewer performance budgets and accessibility
milestone: M5 · The map
labels: type:infra, area:viewer, P1, risk:medium, size:M, lab-evidence
---
**Why.** Lab frame times: 16.67 ms on an RTX 4090 (vsync-capped, so no headroom figure), 29–136 ms mean and 567 ms worst with software rendering. Without budgets enforced in CI the viewer will regress on the hardware most users have.

**Scope**
- [ ] Reference configurations and budgets from {{#m1-12-viewer-no-gpu}}, all on this host (`design/DECISIONS.md`, "Usability, accessibility and hardware checks"): the upper bound is the RTX 4090, 60 fps; the lower bound is software rendering (SwiftShader and Mesa llvmpipe), "usable" with reduced detail; the middle is Chromium with its GPU and CPU throttling flags, with one configuration chosen to stand for an integrated GPU, ≥ 30 fps. The flags are recorded with every result. The throttled configuration is a model of an integrated GPU, not a measurement of one
- [ ] Automated frame-time tests with Playwright (uncapped frame timing, scripted zoom/pan). Software rendering and the throttled configurations run in a GitHub-hosted scheduled workflow within the free quota; the RTX 4090 runs in the coordinator's official window on this host
- [ ] Adaptive detail: instance and edge caps driven by measured frame time
- [ ] Bundle size budget for the viewer (target ≤ 150 KB gzip)
- [ ] Accessibility: keyboard navigation, contrast, colour-blind-safe defaults, reduced-motion setting. The screen-reader route is no longer a summary of the current view — a summary is alt text, which fails (`docs/research/sota-human-factors.md` § 2) — but the navigable hierarchy of {{#m5-16-nonvisual-navigation}}, split out of this issue on 2026-11-25 (`docs/research/README.md` H2)
- [ ] A WCAG 2.2 AA checklist in CI (decided 2026-11-25, `docs/research/README.md` H5; `docs/research/sota-human-factors.md` challenge 5), Playwright beside the frame tests: 1.4.11 non-text contrast on the dark canvas (region outlines of 1.5 px at 12%/5% tint and focus rings need 3:1; not measured yet), 2.5.7 panning without dragging (keys or buttons), 1.4.10 reflow at 320 CSS px, 4.1.3 status messages (tile loading, "shown X of Y"), 2.3.3 reduced motion, 200% browser zoom scaling the map's text; axe-core plus a checklist for what it cannot see, worked through by an agent-driven session from screenshots and the accessibility tree. Run it on `design/preview/style/viewer.html` first, to list the failures the build must fix
- [ ] Browser matrix through Playwright: Chromium and Firefox (headless), mobile by Playwright's device emulation; Safari (WebKit) through Playwright if its system libraries are present on this host, otherwise advisable after 0.1, not a gate
- [ ] The measurements {{#m1-12-viewer-no-gpu}} could not take: frame cost and frame rate of the lab viewer at the two bounds (SwiftShader and Mesa llvmpipe; the RTX 4090) and on the throttled Chromium configurations, in the browsers of the matrix, with `lab/m1-12-viewer-no-gpu/measure_viewer.mjs`; confirm or change the caps and the adaptive control found on SwiftShader. A run on a real integrated-GPU or Apple-silicon machine is advisable after 0.1, not a gate

**Acceptance**
- ≥ 30 fps on the throttled Chromium configuration that stands for the integrated-GPU reference machine of {{#m1-12-viewer-no-gpu}}, and "usable" with reduced detail under both SwiftShader and Mesa llvmpipe
- CI fails when a scripted session exceeds the frame budget on the reference configurations
- CI fails when an automated WCAG 2.2 AA check of the list above fails, axe-core included

**Depends on** {{#m5-05-viewer-core}}, {{#m1-12-viewer-no-gpu}} (the reference machines and budgets)

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Contrast, focus, hit-target and colour-independence requirements are stated and measured in `design/preview/style/foundations.html` (§9) and `data.html` (§1–2); the viewer must meet the same figures.
