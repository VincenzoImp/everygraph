---
title: Correctness oracle: differential and invariant tests
milestone: M0 · Foundations and decisions
labels: type:infra, area:foundations, P0, risk:medium, size:L
---
**Why.** One wrong number destroys trust permanently, and this engine will produce the same answer through several code paths (regimes, codecs, thread counts, native/wasm).

**Scope**
- [ ] Reference implementations as test oracles: NetworkX, python-igraph and scipy for every kernel on graphs ≤ 10^5 edges
- [ ] Property-based generators (proptest): random, power-law, star, path, clique, disconnected, with self-loops/multi-edges/isolates, directed and undirected
- [ ] Conservation invariants checked after every transformation: Σ degree = 2m, Σ quotient weights = m, Σ community sizes = n, symmetric adjacency, sorted lists
- [ ] Cross-path equality: same kernel through in-memory slices, mmap and streams, raw and compressed, 1 and N threads, must give identical bytes (or documented tolerance for floating point)
- [ ] Golden files for the real tier-1 datasets (triangle counts, core numbers, component counts)
- [ ] Semantics table: where NetworkX and igraph disagree with each other, write down which one we follow and why

**Acceptance**
- Oracle suite runs in CI in < 10 minutes
- A seeded bug (off-by-one in the last list of a block) is caught by at least two independent checks

**Pointers.** Lab validations: FINDINGS § B1 (byte-identical CSR, known triangle counts), § B2 (independent numpy check of aggregates), § F2 (answers checked against igraph).

**Depends on** {{#m0-01-workspace}}
