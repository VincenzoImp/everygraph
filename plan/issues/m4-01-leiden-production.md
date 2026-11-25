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
- Sequential aggregation as the only aggregation, in every regime (added 2026-11-25 from {{#m1-02-ooc-aggregation}}, PR #143, FINDINGS § M1-02): one streaming pass in node order into range-partitioned buckets, then per-bucket sort and combine (`lab/m1-02-ooc-leiden/src/seqagg.rs`). In RAM it is faster than member-order aggregation: Friendster, async, 1 iteration, 16 threads, f64 weights, compressed level 0, idle — 74.8 s (74.4–75.5, 3 repetitions; the out-of-core code path with `--budget-gb 40` under 48 GiB, warm, nothing spilled; `bench/results/ooc-leiden/20261125T174959Z-b61036e59740.jsonl`, case `seqagg-inram-friendster`) against 100.1 s for member-order `--lowmem` (2 repetitions; `…20261125T160906Z-e501e2cf5727.jsonl`, case `inram-friendster`); level-0 aggregation 16.2 s against 28.5 s. M1-13's default member-order aggregation took 82.0 s with no limit and is OOM-killed under 48 GiB (`lab/m1-02-ooc-leiden/results/default_agg_oom.jsonl`, exploratory). Its cost: about 29.0 GiB anonymous peak in RAM against 23.75 GiB for `--lowmem`, because the buckets (27.3 GB) are held in RAM when the budget allows; the accountant decides how many go to files. The deterministic partition does not depend on the aggregation: member-order in RAM and sequential out of core give the same hash on Friendster (`2a12e5882874c38d`, official) and on lj and orkut (`lab/m1-02-ooc-leiden/results/determinism.jsonl`, exploratory). One implementation then serves in-RAM and out-of-core runs ({{#m4-04-ooc-leiden}})

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
- [ ] The pass-stopping test leaves out vertices of strength 0: counting isolated vertices stopped the lab solve early on perturbed graphs (lj with a 1% edge drop: 9,559 communities instead of about 1,363; orkut 702 instead of 29) and flipped the map's super-level decision (#19, PR #162)

**Acceptance (lab machine, idle, 16 threads)**
- `fast`: lj ≤ 0.8 s, orkut ≤ 1.5 s, modularity ≥ lab values
- `deterministic`: identical hash at 1, 2, 4, 8, 16 threads on all tier-1/2 graphs; time ≤ 2× `fast` (stretch 1.5×, pending {{#m1-04-det-leiden-scaling}})
- 0 disconnected communities on every run
- No data race reported by TSan
- On every oracle graph, kept loops included, the modularity of each level's partition equals the modularity of its quotient graph under the singleton partition (`docs/spec/graph-semantics.md` § 5.5)

**Pointers.** `lab/b2-leiden/src/main.rs`, FINDINGS § B2 and idle re-runs.

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Repeatable results are the default (`api.md` Choice 6, approved); the fast mode is opt-in and labelled in the output. The method is exposed as `g.communities()`, not `g.leiden()`.
