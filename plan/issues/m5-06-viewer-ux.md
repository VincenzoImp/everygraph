---
title: Viewer experience: inspect, search, colour, share
milestone: M5 · The map
labels: type:feature, area:viewer, P0, risk:medium, size:L
---
**Why.** The viewer is the product for the analyst persona. The lab prototype has hover and fly-to; a usable tool needs more, and it must keep the "exact numbers, declared omissions" discipline visible.

**Scope**
- [ ] Selection panel for a region or node: exact counts, strongest neighbours with weights, path in the hierarchy, node properties; "shown X of Y" wherever something is omitted
- [ ] Search box with exact and prefix search, keyboard navigation, fly-to
- [ ] Breadcrumb of the hierarchy and "zoom to parent / siblings"
- [ ] Colour modes: top-level region, any level, a node property, PageRank/degree; colour-blind-safe palettes; legend
- [ ] URL state: position, zoom, selection and colour mode encoded in the link, so a view can be shared
- [ ] Screenshot/export of the current view as PNG and SVG with caption and scale
- [ ] Empty/error states: slow network, unsupported browser, corrupt file
- [ ] Keyboard accessibility for all actions, by the keyboard model decided 2026-11-25 (`design/DECISIONS.md`, "The map's keyboard model"; `docs/research/README.md` H3): arrows move through the structure as `foundations.html` § 9 says, Shift+arrows pan, `/ + - 0` act only while the map has focus. The preview `viewer.html` (arrows pan, `/ + - 0` bound at document level, around line 1865) is to be updated to this model when this issue is built

**Acceptance**
- Agent-driven usability check (`design/DECISIONS.md`, "Usability, accessibility and hardware checks"): Playwright drives the viewer, and 5 Claude agent sessions, each briefed as one of the target personas, do three tasks (find a node, describe its community, find what connects two regions) from screenshots and the accessibility tree, without help. ≥ 4 of 5 sessions succeed on each task. Each session records task success, steps, time and errors. The time counts each action at the operator times of the keystroke-level model (Card, Moran and Newell, 1980), not at the agent's own response time
- A heuristic evaluation of the same sessions against Nielsen's ten usability heuristics, and axe-core on every state the tasks reach; failures are filed as issues
- This is formative, not a claim (`docs/research/README.md` H4): 4 of 5 has an exact 95% lower bound of 0.28 (Clopper–Pearson), and an agent session is not evidence of what people do. A study with people from the personas is advisable after 0.1, never a gate. A success rate is published only from such a study, with n ≥ 20 per task and its exact interval (18 of 20 → lower bound 0.68)
- Every number shown can be traced to a value in the file

**Depends on** {{#m5-05-viewer-core}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Approved as shown in `design/preview/style/viewer.html` and its 21 captured states: plan card before starting, breadcrumb and level indicator, inspection panel (kind, exact counts, ranked strongest connections with exact weights, actions), search with fly-to, path with an "avoid hubs" toggle off by default, time strip opened on demand, colour by region / attribute / metric with a legend, share dialog, compact density as a preference, and the caption strip always visible. Components and their states are in `components.html`.
