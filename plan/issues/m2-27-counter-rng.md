---
title: Counter-based RNG in the core, with a written key schema
milestone: M2 · Format and ingest
labels: type:feature, area:engine, P1, risk:low, size:S
---
**Why.** Every randomized step — the seeded relabelling, Leiden's visiting order and refinement θ, betweenness samples, label-propagation ties, layout jitter, WCC's k-out sample — must give the same draws for any thread count, budget and regime (D14, D15). A counter-based generator makes that true by construction: the n-th draw is a function of (key, n), with no state and no sequence (Salmon et al., SC'11). The lab already keys its draws this way in practice (SplitMix64 on (seed, node) in `lab/m1-02-ooc-leiden/src/main.rs` l. 111–125; (seed, parent, tile) in `lab/e-map/gmap/src/layout.rs` l. 428), but the policy is not written, and nothing stops the next kernel from using `thread_rng` or a stream advanced in schedule order. Leiden's sub-round priority hash is unseeded (`x.wrapping_mul(0x85EB_CA6B)`), so seeds do not vary it, which matters for consensus across seeds ({{#m9-10-statistical-rigor}}). `rand`'s `StdRng` and `SmallRng` are not portable across versions and platforms, and sampling a `usize` differs between 32- and 64-bit targets (rand book, "Reproducibility"). Decided 2026-11-25 (`docs/research/README.md` N5).

**Scope**
- [ ] One function in `everygraph-core`, `rng(seed, purpose, counter) -> u64`, implemented as Philox4×32-10 (about 40 lines, integers only; 3.6 cycles per byte on one Xeon X5667 core in Salmon et al., Table 2, their machine; it passes BigCrush)
- [ ] A written key schema: seed, a purpose tag per use, the entity ids (node, community, sample index), level, iteration or round, draw index. Thread id, chunk index, budget, regime and any stream advanced in schedule order never enter the key. Documented next to the function and cited by every kernel that draws
- [ ] Conversions: floats as `(x >> 11) as f64 · 2^-53`; bounded integers by a 64-bit multiply-shift on `u64`, never on `usize`; no `rand_distr` float distributions
- [ ] Frozen reference vectors, checked on x86-64, aarch64, i686 and wasm32 in {{#m3-11-determinism-ci}}
- [ ] The ban of `thread_rng`, `rand::rng`, `StdRng`, `SmallRng`, `fastrand` and `getrandom` in result crates (rule R9 of {{#m3-11-determinism-ci}}, enforced by {{#m2-18-portability-lint-shared}} and `deny.toml`)
- [ ] Leiden's sub-round priority hash takes the seed (a note for {{#m4-01-leiden-production}})

**Acceptance**
- Reference vectors bit-identical on the four targets
- A Leiden hash at two seeds differs; at one seed it is equal across 1, 4 and 16 threads (once {{#m4-01-leiden-production}} uses it)

**Feeds.** {{#m3-11-determinism-ci}}, {{#m4-01-leiden-production}}, {{#m1-14-relabel-at-ingest}}, {{#m3-09-centrality}} (samples keyed by sample index), {{#m4-09-label-propagation}}, {{#m5-01-layout}}, {{#m3-05-components}}.

**Pointers.** `docs/research/sota-numerics.md` § 3.1 (D15 / m1-14 and ADR 0008 rows), § 4 challenge 5, rule R9; `docs/research/sota-approx-rust.md` § 3 A (betweenness row); Salmon, Moraes, Dror, Shaw, SC 2011.
