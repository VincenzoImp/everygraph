---
title: M4 performance pass: close the measured speed gaps of community detection
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:medium, size:L
---
**Why.** As in {{#m3-22-performance-pass}}: issues close on correctness and exploratory evidence, and their missed speed targets come here. All numbers below are exploratory (interleaved A/B on a loaded machine) unless a result file is named.

**Gaps to close**
- [ ] Production Leiden ({{#m4-01-leiden-production}}, PR #232) against `lab/b2-leiden` at 16 threads, raw CSR, same rule L64k: deterministic 1.27–1.59×, fast 1.4–1.9× (lj and orkut, 1 and 3 iterations). Largest known cause: the edge-map framework's fixed cost per frontier run of 256 nodes (four lock operations; the fast mode runs about 15,600 runs per level-0 sweep on lj), shared with #44's per-task overhead in {{#m3-22-performance-pass}}. Then local moving (1.16× b2 at 8 threads), refinement and numbering (1.7×), aggregation (1.3×).
- [ ] The time criteria of {{#m4-01-leiden-production}}: `fast` lj ≤ 0.8 s and orkut ≤ 1.5 s; `deterministic` ≤ 2× `fast` (stretch 1.5×) at 16 threads.

**Done when.** Each gap is closed (interleaved A/B against `lab/b2-leiden` and the same-machine references of {{#m4-05-quality-gates}}, ratio stated) or its remaining cause is written down with a follow-up; the milestone-end official run (`everygraph-bench run leiden --official`) records the final numbers.

**Depends on** {{#m4-01-leiden-production}}
