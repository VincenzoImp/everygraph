---
title: Graph semantics: direction, self-loops, multi-edges, weights, time
milestone: M2 · Format and ingest
labels: type:spec, area:ingest, P0, risk:medium, size:M
---
**Why.** The old prototype accepted only undirected, unweighted, simple graphs, which excluded the author's own data (transactions, forwards, follows). The semantics must be explicit, counted and reported — silently dropping edges is how numbers stop being trusted.

**Scope**
- [ ] Graph kinds: undirected, directed; simple or multigraph
- [ ] Self-loops: keep / drop (counted either way)
- [ ] Parallel edges: keep as events, collapse to one, or aggregate (count, sum/min/max of a weight, first/last timestamp)
- [ ] The event/projection model from the lab's reasoning: store one entry per event sorted by (neighbor, timestamp) so parallel edges are contiguous, and derive a simple-graph projection (unique neighbor, multiplicity, aggregated weight) for analytics
- [ ] Weight types: f32/f64 and exact integer/decimal columns (token amounts do not fit in f32)
- [ ] Edge types as a small categorical column
- [ ] An ingest report: rows read, rejected, self-loops, duplicates, isolates, resulting n and m — stored in the file's provenance
- [ ] What each kernel does on each kind (table), consistent with {{#m0-05-oracle}}

**Acceptance**
- Spec in `docs/spec/graph-model.md`; every option covered by oracle tests
- The ingest report sums exactly to the input row count
