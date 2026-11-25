---
title: Production Leiden: deterministic and fast modes
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P0, risk:high, size:XL, lab-evidence
---
**Why.** Community detection is the heart of the product and the lab prototype (~1,900 lines) is fast but research-grade: shared-memory `unsafe`, lightly exercised options, one known bug (`--cap` with multiple iterations).

**Keep from the lab**
- Block-permuted visiting order (blocks of 64 ids): worth 3–4 modularity points over id order; quality is flat for block sizes 1–1,024
- Split-last step for a real connectivity guarantee (it fired: +27 communities on lj, +476 on Friendster — GVE-style refinement alone is not enough)
- Deterministic mode: hashed sub-rounds, frozen decisions, commutative commit, smaller-label merge rule, fixed-point gain sums, deterministic refinement and aggregation
- Threshold scaling, flag pruning, per-thread open-addressing tables, f64 quotient weights (f32 is exact only to 16.7 M)

**Scope**
- [ ] Port onto the edge-map framework ({{#m3-01-edge-map}}) so level 0 runs over compressed and streamed sources
- [ ] Safety: encapsulate the shared-state `unsafe`, Miri on small graphs, ThreadSanitizer in CI
- [ ] Modes: `deterministic` (default) and `fast`; iterations 1 (default) and 3 ("quality"), plus convergence-based stopping
- [ ] Apply the fixes from {{#m1-03-louvain-collapse}} and {{#m1-04-det-leiden-scaling}}
- [ ] Seeds: the visiting-order permutation is seeded; same seed → same result
- [ ] Louvain kept as an internal baseline, not a public default (17–94 disconnected communities per run)
- [ ] Outputs: membership column per level, community table (size, internal edges, volume, cut), modularity

**Acceptance (lab machine, idle, 16 threads)**
- `fast`: lj ≤ 0.8 s, orkut ≤ 1.5 s, modularity ≥ lab values
- `deterministic`: identical hash at 1, 2, 4, 8, 16 threads on all tier-1/2 graphs; time ≤ 2× `fast` (stretch 1.5×, pending {{#m1-04-det-leiden-scaling}})
- 0 disconnected communities on every run
- No data race reported by TSan

**Pointers.** `lab/b2-leiden/src/main.rs`, FINDINGS § B2 and idle re-runs.

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Repeatable results are the default (`api.md` Choice 6, approved); the fast mode is opt-in and labelled in the output. The method is exposed as `g.communities()`, not `g.leiden()`.
