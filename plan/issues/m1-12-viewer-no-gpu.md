---
title: The viewer without a discrete GPU, and on Firefox and Safari
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:viewer, P1, risk:medium, size:S, lab-evidence
---
**Question.** The viewer held 16.67 ms frames on an RTX 4090 (vsync-capped, so no headroom figure). With software rendering it averaged 29–136 ms and peaked at 567 ms. Most users have an integrated GPU. What budgets make it usable there?

**Method**
- [ ] Measure on at least one Intel/AMD integrated GPU and one Apple-silicon laptop, uncapped frame time while zooming and panning
- [ ] Find the per-frame instance and edge caps that hold 30 fps; implement adaptive caps in the lab viewer
- [ ] Check Firefox and Safari (WebGL2 behaviour, range requests, worker support); the lab only ran headless Chromium
- [ ] Decide the minimum supported configuration and the fallback message

**Done when.** Frame-time tables for the three hardware classes and three browsers, the per-frame caps that hold 30 fps on integrated graphics, and a stated minimum supported configuration.

*Amended 2026-11-26.* This host has a discrete GPU, a software renderer (SwiftShader) and headless Chromium and Firefox; it has no integrated GPU, no Apple-silicon laptop and no Safari (WebKit needs system libraries that cannot be installed without `sudo`), and headless Firefox cannot create WebGL here. M1 closes on what the host can answer, labelled exploratory: frame cost and frame rate on the RTX 4090 and on SwiftShader, the caps and the adaptive control that hold 30 frames per second on SwiftShader, first-view bytes, the minimum configuration and the message without WebGL 2 (`lab/m1-12-viewer-no-gpu/results/`). The tables on an integrated GPU, an Apple-silicon laptop, Safari and Firefox with a display move to {{#m5-11-perf-accessibility}}; they need hardware this host does not have (CLAUDE.md § 8).

**Feeds.** Performance budgets in {{#m5-11-perf-accessibility}}.
