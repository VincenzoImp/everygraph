---
title: Resume inside the long map phases: hierarchy subtrees and tile ranges
milestone: M9 · After v0.1
labels: type:feature, area:map, P2, risk:medium, size:M
---
**Why.** Each phase already stores its result in the graph file, so an interrupted build reuses finished phases. A phase that runs for hours (the hierarchy of a 10 B-edge graph, its tiles) restarts from its beginning.

**Scope**
- [ ] Checkpoints inside a phase (finished subtrees, finished tile ranges), verified on restart and invalidated by any change of input, options or version
- [ ] The final artefact is byte-identical with and without a restart (D14)
