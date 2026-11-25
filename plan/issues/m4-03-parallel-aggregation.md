---
title: Aggregation rewrite: the scaling bottleneck
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P1, risk:medium, size:M, lab-evidence
---
**Why.** Aggregation takes 35–40% of Leiden's time at 16 threads on lj/orkut and 58–69% on Friendster (89.5 of 154.5 s, 32 threads, shared machine; #118 measured one async iteration at 16 threads idle in 82.4 s raw / 82.0 s compressed, without a phase breakdown). The asynchronous code scales 8–11× from 1 to 16 threads; aggregation is what holds it back.

**Scope**
- [ ] Parallel prefix sums and buffer reuse across passes (identified, not done, in the lab)
- [ ] Choose per level between hash-based aggregation (small quotient) and sort/bucket aggregation (large quotient, sequential I/O — shared with {{#m4-04-ooc-leiden}})
- [ ] Avoid the "holey CSR" O(m) temporary by default when the budget is tight (the two-pass `--lowmem` path cut lj's peak from 1.65 to 1.00 GB)
- [ ] Write coarse graphs in the standard adjacency format so every level is a normal graph
- [ ] Quotient weights summed as i64 fixed point ({{#m4-02-weights-resolution}}; `docs/research/README.md` N1), so hash and bucket aggregation may combine in any order and give the same bits; the cost measured first as an interleaved A/B on Friendster level-1 aggregation, f64 against i64 fixed point, 16 threads (`docs/research/sota-numerics.md` challenge 1). The report's expectation, unmeasured: no slower, since an i64 `fetch_add` replaces an f64 CAS loop (x86 has no atomic float add)
- [ ] No `unsafe` scatter copied from the lab's `SyncPtr`: ranged writes and the member scatter go through the safe helpers of {{#m2-28-par-scatter}}; the lab's `fetch_add` member positions give a member order that depends on the schedule and must not reach results

**Acceptance**
- Aggregation ≤ 25% of total time at 16 threads on lj and orkut
- Friendster in-RAM Leiden, one async iteration, 16 threads, idle machine: ≤ 58 s against the idle baseline of 82.4 s (#118, `lab/m1-13-compressed/results/leiden_big.jsonl`) — the same 0.71× ratio as the original target (110 s from 154.5 s, measured on a shared machine at 32 threads, not comparable). Re-measure the baseline in the same configuration when the issue starts; the ratio is the target (decided 2026-11-25)

**Depends on** {{#m4-01-leiden-production}}
