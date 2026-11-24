---
title: BFS, shortest paths and multi-source traversal kernels
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P0, risk:medium, size:M, lab-evidence
---
**Scope**
- [ ] Direction-optimizing BFS (lab parameters α = 15, β = 18; 22 ms on orkut, 0.34 s on Friendster) for directed and undirected graphs
- [ ] Outputs: distance column, parent column, level sizes
- [ ] Weighted SSSP (delta-stepping) on a per-entry weight column
- [ ] Multi-source BFS (bit-parallel) for batches of sources — the basis for sampled centralities and diameter estimates
- [ ] High-diameter graphs (roads): check the level-synchronous design does not collapse; see {{#m1-10-graph-families}}
- [ ] Semi-external variant: bottom-up steps are scans; top-down steps need random list access with frontier prefetch

**Acceptance**
- Distances equal NetworkX on the oracle graphs
- orkut BFS ≤ 30 ms at 16 threads; a road graph completes without per-level fork-join overhead dominating
- Under a memory limit below the adjacency size, BFS on Friendster completes in at most the time of three sequential reads of the graph file (in RAM it takes 0.34 s, so a ratio to in-RAM time is not a meaningful target)

**Depends on** {{#m3-01-edge-map}}
