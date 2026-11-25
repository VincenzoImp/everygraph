---
title: BFS, shortest paths and multi-source traversal kernels
milestone: M3 · Engine and kernels
labels: type:feature, area:algorithms, P0, risk:medium, size:M, lab-evidence
---
**Scope**
- [ ] Direction-optimizing BFS (lab parameters α = 15, β = 18; 22 ms on orkut, 0.34 s on Friendster) for directed and undirected graphs
- [ ] Outputs: distance column, parent column, level sizes
- [ ] Weighted SSSP on a per-entry weight column by **Δ*-stepping or ρ-stepping** (Dong, Gu, Sun, Zhang, SPAA'21) with **bucket fusion** (GraphIt, CGO'20) and Δ chosen from sampled weights by a rule that depends on the data only, never on the thread count (AdaMW's rule uses the thread count; ours must not). A fixed Δ is fragile: "Mind the Gap" (arXiv 2026) shows the optimal Δ moving by orders of magnitude between weight distributions, with mis-tuning costing up to 4.92×. Distances are unique, so only speed varies; parents by the post-pass rule of {{#m7-02-shortest-paths}} (decided 2026-11-25, `docs/research/sota-kernels.md` challenge 8). Measured on natural-weight graphs (a road graph, a weighted social graph) and uniform weights, with a Δ sweep, at 16 threads
- [ ] Multi-source BFS (bit-parallel) for batches of sources — the basis for sampled centralities and diameter estimates
- [ ] High-diameter graphs (roads): check the level-synchronous design does not collapse; see {{#m1-10-graph-families}}
- [ ] On compressed input: degree from the entry-offset index or the traversal state, and bottom-up stops decoding at the first hit (#118: 1.20–1.52× at 16 threads, 1.78–2.04× at 1; with the entry-offset index 0.99–1.22× at 16)
- [ ] Semi-external variant: bottom-up steps are scans; top-down steps need random list access with frontier prefetch

**Acceptance**
- Distances equal NetworkX on the oracle graphs
- orkut BFS ≤ 30 ms at 16 threads (default storage measured 28.0 ms in #118); a road graph completes without per-level fork-join overhead dominating
- Under a memory limit below the adjacency size, BFS on Friendster completes in at most the time of three sequential reads of the graph file (in RAM it takes 0.34 s, so a ratio to in-RAM time is not a meaningful target)

**Depends on** {{#m3-01-edge-map}}
