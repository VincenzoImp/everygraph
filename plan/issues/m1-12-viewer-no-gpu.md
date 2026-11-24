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

**Feeds.** Performance budgets in {{#m5-11-perf-accessibility}}.
