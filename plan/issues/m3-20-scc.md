---
title: Strong components with BGSS on the in-adjacency
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P1, risk:medium, size:M
---
**Why.** `m3-05-components` (#48) built weak components; strong components need the in-adjacency (#38) and were left out of PR #218. The research decision (`docs/research/README.md` A10, `docs/research/sota-kernels.md` § 3) is BGSS (Wang et al., SIGMOD'23), not forward-backward with trimming: forward-backward methods are slower than sequential Tarjan in 24 of 36 large-diameter tests there, and BGSS is 1.7–19× faster than GBBS (their machine).

**Depends on** {{#m2-11-in-adjacency}}, {{#m3-05-components}}

**Scope**
- [ ] BGSS on the out- and in-adjacency through the edge-map framework: prefix-doubling reachability, a deterministic hash table for the reachability sets, labels = smallest node of each component
- [ ] Results identical for any thread count, chunking and source (raw, compressed, mapped, streamed)
- [ ] Oracle against NetworkX/igraph/scipy SCC; metamorphic relations (relabel, disjoint union, arc reversal keeps SCCs)
- [ ] Exploratory interleaved A/B against a sequential Tarjan and, if quick, GBBS; the official command handed back

**Done when.** The oracle and metamorphic tests pass in CI, and SCC runs on twitter-2010 and one large-diameter graph (a road network), with its time from an exploratory interleaved run; the official time comes from the M3 milestone window (CLAUDE.md § 6).

**Pointers.** `crates/everygraph-kernels/src/components.rs` (WCC, PR #218); `docs/research/sota-kernels.md`.
