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
- [ ] Progress display for the build ({{#m0-12-ux-contract}})

**Acceptance**
- 1 M-edge CSV → inline map in < 10 s in a fresh notebook on a laptop

**Depends on** {{#m5-05-viewer-core}}, {{#m6-01-python-api}}
