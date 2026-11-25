---
title: Production Leiden: deterministic and fast modes
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P0, risk:high, size:XL, lab-evidence
---
**Why.** Community detection is the heart of the product and the lab prototype (~1,900 lines) is fast but research-grade: shared-memory `unsafe`, lightly exercised options, one known bug (`--cap` with multiple iterations).

**Keep from the lab**
- Block-permuted visiting order (blocks of 64 ids): worth 2.0–2.1 modularity points over id order at one iteration on an idle machine (lj, orkut; 2.6–4.2 in the first shared runs); quality is flat for block sizes 1–1,024
- Split-last step for a real connectivity guarantee (it fired: +27 communities on lj, +476 on Friendster — GVE-style refinement alone is not enough)
- Deterministic mode: hashed sub-rounds, frozen decisions, commutative commit, smaller-label merge rule, fixed-point gain sums, deterministic refinement and aggregation
- Threshold scaling, flag pruning, per-thread open-addressing tables, f64 quotient weights (f32 is exact only to 16.7 M)

**Scope**
- [ ] Port onto the edge-map framework ({{#m3-01-edge-map}}) so level 0 runs over compressed and streamed sources
- [ ] Safety: encapsulate the shared-state `unsafe`, Miri on small graphs, ThreadSanitizer in CI
- [ ] Modes: `deterministic` (default) and `fast`; iterations 1 (default) and 3 ("quality"), plus convergence-based stopping
- [ ] The default number of iterations is confirmed or changed with evidence: quality and time at 1, 2, 3 iterations and at convergence on the six graph families, recorded in FINDINGS; a change of default goes through an ADR (`design/DECISIONS.md`, 2026-11-25)
- [ ] Apply the fixes from {{#m1-03-louvain-collapse}} and {{#m1-04-det-leiden-scaling}}
- [ ] Seeds: the visiting-order permutation is seeded; same seed → same result
- [ ] Louvain kept as an internal baseline, not a public default (17–94 disconnected communities per run)
- [ ] Outputs: membership column per level, community table (size, internal edges, volume, cut), modularity
- [ ] Modularity with kept loops as `docs/spec/graph-semantics.md` § 5.5 (S5): `Q = Σ_c [L(c)/W − γ(S(c)/2W)²]`, L(c) counting each loop once, s(v) including the loop's 2w, Q = 0 when W = 0; the aggregation's quotient carries the input's kept loops on its diagonal with the internal edges (§ 5.5, S6)

**Acceptance (lab machine, idle, 16 threads)**
- `fast`: lj ≤ 0.8 s, orkut ≤ 1.5 s, modularity ≥ lab values
- `deterministic`: identical hash at 1, 2, 4, 8, 16 threads on all tier-1/2 graphs; time ≤ 2× `fast` (stretch 1.5×, pending {{#m1-04-det-leiden-scaling}})
- 0 disconnected communities on every run
- No data race reported by TSan
- On every oracle graph, kept loops included, the modularity of each level's partition equals the modularity of its quotient graph under the singleton partition (`docs/spec/graph-semantics.md` § 5.5)

**Pointers.** `lab/b2-leiden/src/main.rs`, FINDINGS § B2 and idle re-runs.

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Repeatable results are the default (`api.md` Choice 6, approved); the fast mode is opt-in and labelled in the output. The method is exposed as `g.communities()`, not `g.leiden()`.
