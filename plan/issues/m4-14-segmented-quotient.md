---
title: Leiden: segmented per-level columns and quotient graphs (D2)
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P2, risk:medium, size:M
---
**Why.** DESIGN D2 asks for arrays in segments of at most 1 GiB (`MAX_ALLOCATION_BYTES`), so the same code runs on wasm32. In {{#m4-01-leiden-production}} every per-level column and the quotient graph (`offsets`, `targets`, `weights`) is one allocation: the coarse level is a `Csr`, which takes contiguous slices. At Friendster level 1 the quotient holds about 1.4 G entries (about 5.6 GB of targets and 11 GB of weights, estimate from the entry count). On a 64-bit target that works in RAM; on a 32-bit target a column above 1 GiB is refused with `Error::Memory` (`crates/everygraph-community/src/column.rs`).

**Scope**
- [ ] A source of the edge-map framework over segmented targets and offsets (`SegVec`), or the coarse levels written in the compressed format, shared with the out-of-core path of {{#m4-04-ooc-leiden}} and {{#m4-03-parallel-aggregation}}.
- [ ] Per-entry weights indexed across segments without a copy per list.
- [ ] Per-node columns in `SegVec` where a node count can pass 2^28.

**Done when.** No single allocation of Leiden exceeds 1 GiB on any target, the deterministic hash is unchanged, and an interleaved A/B shows the cost on lj and orkut.

**Depends on** {{#m4-01-leiden-production}}
