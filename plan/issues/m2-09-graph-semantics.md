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
- [ ] The plan reports the treatment chosen with the values of `docs/ux-contract.md` § 3.6: `self_loops` `dropped` or `kept`; `duplicates` `kept as events`, `collapsed` or `summed`. The public options are those of `docs/api-sketch.md` (ingest options), with the approved defaults: self-loops dropped and counted, duplicates kept as events
- [ ] Settle the two cases the oracle ({{#m0-05-oracle}}, `oracle/SEMANTICS.md` Q2) leaves unchecked because the reference libraries disagree: PageRank and core numbers on a graph with kept self-loops. Define them, add them to the kernel table, and turn the oracle's "not checked" into a check. The other answers are fixed (`design/DECISIONS.md`, 2026-11-25, "Graph semantics")

**Acceptance**
- Spec in `docs/spec/graph-model.md`; every option covered by oracle tests
- The ingest report sums exactly to the input row count

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Defaults approved in `design/preview/choices.md` (A1–A4): repeated edges are kept as separate events and analysed once per pair with a weight; self-loops are dropped and counted; direction is taken from the caller, or inferred from column names (`from`/`to`, `source`/`target`) with a line in the summary; community detection ignores direction by default and says so.
