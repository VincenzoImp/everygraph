---
title: Weights, resolution and objectives
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:medium, size:M, lab-evidence
---
**Why.** Real graphs are weighted, and one resolution rarely fits. The lab only checked that all-1.0 weights reproduce the unweighted hash and that γ ∈ {0.25, 1, 4, 16} behaves sensibly on lj.

**Scope**
- [ ] Edge weights from a per-pair column (the pair sum, indexed by entry offset; kernels never read per-event columns, ADR 0028); integer weights exact
- [ ] Fixed-point weights for every sum whose order depends on the schedule (decided 2026-11-25, `docs/research/README.md` N1; `docs/research/sota-numerics.md` § 3.3 option B, challenge 1): community volumes σ, quotient weights in aggregation, L(c), S(c) and level totals are i64. Each weight w becomes round(w·2^s), with s chosen once from the exact total T of all entry weights so that T·2^s ≤ 2^62; integer sums are exact in any order, so the result does not depend on threads, budget or regime. Unweighted and integer-weighted graphs with T < 2^62 get s ≥ 0 and keep today's bits exactly; the gain stays an f64 expression of exactly converted integers. `weight_scale` (s) is recorded in the result and its provenance. Inputs whose total and smallest non-zero weight do not fit 62 bits get a two-limb i64 (Hallberg–Adcroft) or i128 fallback, or a refusal that says why. Quantization is documented: every weight is a multiple of 2^-s; the worst-case total error is about 2^-30·T on Friendster's 3.6·10^9 entries, against 2^-21·T for an f64 recursive sum (arithmetic in the report). Not a fixed summation order: it would bind every future aggregation ({{#m4-03-parallel-aggregation}}) and the atomic commit to one order that a single optimization breaks silently (Müller et al.: sorting to fix the order cost 7× CPU in MonetDB, their machine), while integer sums cost what float sums cost (Müller et al., Fig. 4) and binned accumulators 1.9–2.4× and 48 B each
- [ ] The lab's σ update, an f64 `compare_exchange_weak` loop (`lab/b2-leiden/src/util.rs` l. 51–56), becomes an i64 `fetch_add`: exact for any weights, not only for integer degrees
- [ ] Resolution parameter γ for modularity; Constant Potts Model as a second objective (resolution-limit free, and the natural generator for nested levels in {{#m1-05-hierarchy-shaping}})
- [ ] Directed graphs: default to the symmetrized weight (documented), optional directed modularity — decision informed by {{#m1-10-graph-families}}
- [ ] Multigraph projection weights (edge multiplicity) from {{#m2-09-graph-semantics}}
- [ ] Objectives with kept loops as `docs/spec/graph-semantics.md` § 5.5: modularity with L(c) counting each loop once and its 2w in S(c); directed modularity (`use_direction=True`) in the Leicht–Newman form `Q_d = (1/W)·Σ_ij [A_ij − γ·s⁺_i·s⁻_j/W]·δ(c_i, c_j)` with A[v][v] = w; CPM `Σ_c [L(c) − γ·n_c(n_c − 1)/2]` with loops in L(c); `use_direction=True` on an undirected graph is `Unsupported` (2), a negative, NaN or ∞ weight `InvalidInput` (3) (§ 7)

**Acceptance**
- Weighted results match `leidenalg` within tolerance on oracle graphs (same objective and γ)
- Deterministic hash stable across thread counts and across two memory budgets, in RAM and out of core ({{#m4-04-ooc-leiden}}), with random real weights: f32, and log-uniform over 2^-20…2^20 on orkut (`docs/research/sota-numerics.md` challenge 1)
- Modularity, Leicht–Newman and CPM values equal the oracle's on graphs with kept loops, and modularity is equal across aggregation (a partition and its quotient under singletons), also with real weights — attainable only with exact sums, which the fixed-point weights above give: a float sum over members is not in general the same float sum over a quotient (`docs/spec/graph-semantics.md` § 5.5)

**Depends on** {{#m4-01-leiden-production}}, {{#m2-10-property-columns}}, {{#m1-10-graph-families}}
