---
title: Weights, resolution and objectives
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:medium, size:M, lab-evidence
---
**Why.** Real graphs are weighted, and one resolution rarely fits. The lab only checked that all-1.0 weights reproduce the unweighted hash and that γ ∈ {0.25, 1, 4, 16} behaves sensibly on lj.

**Scope**
- [ ] Edge weights from a per-entry column; integer weights exact
- [ ] Fixed-point community totals so deterministic mode stays bit-identical with real-valued weights (designed in the lab, not implemented)
- [ ] Resolution parameter γ for modularity; Constant Potts Model as a second objective (resolution-limit free, and the natural generator for nested levels in {{#m1-05-hierarchy-shaping}})
- [ ] Directed graphs: default to the symmetrized weight (documented), optional directed modularity — decision informed by {{#m1-10-graph-families}}
- [ ] Multigraph projection weights (edge multiplicity) from {{#m2-09-graph-semantics}}

**Acceptance**
- Weighted results match `leidenalg` within tolerance on oracle graphs (same objective and γ)
- Deterministic hash stable across thread counts with random f32 weights
