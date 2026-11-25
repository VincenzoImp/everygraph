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
- [ ] Objectives with kept loops as `docs/spec/graph-semantics.md` § 5.5: modularity with L(c) counting each loop once and its 2w in S(c); directed modularity (`use_direction=True`) in the Leicht–Newman form `Q_d = (1/W)·Σ_ij [A_ij − γ·s⁺_i·s⁻_j/W]·δ(c_i, c_j)` with A[v][v] = w; CPM `Σ_c [L(c) − γ·n_c(n_c − 1)/2]` with loops in L(c); `use_direction=True` on an undirected graph is `Unsupported` (2), a negative, NaN or ∞ weight `InvalidInput` (3) (§ 7)

**Acceptance**
- Weighted results match `leidenalg` within tolerance on oracle graphs (same objective and γ)
- Deterministic hash stable across thread counts with random f32 weights
- Modularity, Leicht–Newman and CPM values equal the oracle's on graphs with kept loops, and modularity is equal across aggregation (a partition and its quotient under singletons) (`docs/spec/graph-semantics.md` § 5.5)
