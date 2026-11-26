---
title: Jupyter widget
milestone: M6 · Python and interop
labels: type:feature, area:viewer, P1, risk:low, size:S
---
**Why.** The primary persona lives in notebooks; a map one line away from the analysis is the shortest path to the "first map" metric.

**Scope**
- [ ] `anywidget`-based widget wrapping the viewer; `g.map()` displays inline
- [ ] Two-way selection: clicking a region gives its node ids in Python; Python can highlight a set of nodes or a result column on the map
- [ ] Works in JupyterLab, VS Code notebooks and Colab (where loopback servers are restricted — use the widget transport, not HTTP)
- [ ] The widget and the cards take styles, fonts and icons from `@everygraph/style` (`design/system/`, {{#m5-14-design-system}}), and the cards' plain-text form reads its styles from `design/system/tokens.json`; no literal colour in the widget's stylesheets (`design/system/tools/lint_colours.py` passes)
- [ ] Progress display for the build ({{#m0-12-ux-contract}})
- [ ] The cards of `docs/ux-contract.md` § 7, driven only by the events of § 4.6; interrupting the kernel ends the card in the stop block
- [ ] The map's keyboard model (`design/DECISIONS.md`, "The map's keyboard model", 2026-11-25): Esc means "up a level" in the map and "command mode" in JupyterLab, so Esc at the top level, or a documented key, leaves the map's focus; the keyboard is never trapped (WCAG 2.1.2); `/ + - 0` act only while the map has focus

**Acceptance**
- 1 M-edge CSV → inline map in < 10 s in a fresh notebook on a laptop

**Depends on** {{#m5-05-viewer-core}}, {{#m6-01-python-api}}, {{#m5-14-design-system}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Look and contents of the inline map and of the cards around it are in `design/preview/style/notebook.html`: styled cards in notebooks with the same content as plain text in a terminal, the traceback folded under the explanation, the user's own nouns in summaries.
