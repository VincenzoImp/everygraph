---
title: "Experiment: power-diagram cells against nested discs, measured end to end"
milestone: M9 · After v0.1
labels: type:experiment, area:map, P2, risk:low, size:M
---
**Why.** m1-07 kept nested discs (`lab/m1-07-layout-area/results/README.md`): cells cost 2.4–2.9× the layout time and lowered local metrics, but gave 8–11× fewer tiles and a 3.1–3.4× smaller tile set. Only the layout was timed. For very large graphs, static export, hosting and phones, the whole pipeline may favour cells.

**Scope**
- [ ] With the production tile builder: layout + tiles + artefact size + first view + a scripted session, discs against cells, on lj, orkut and Friendster
- [ ] Decide whether a "cells" export mode is worth its code

**Depends on** {{#m5-03-tile-builder}}
