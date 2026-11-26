---
title: Read-only GQL / openCypher by bridging to an embedded engine
milestone: M9 · After v0.1
labels: type:feature, area:query, P2, risk:high, size:XL
---
**Why.** Analysts coming from Neo4j expect pattern matching. Building a query engine is years of work, and embedded engines already exist. So the plan is to choose one embedded engine and bridge it to our graphs, and only if users ask for it. Building our own engine (a parser, DataFusion for the relational operators and our operators for expansion) is not an option any more (decided 2026-11-26, `docs/research/sota-review-2026-11.md` S13; ADR 0021).

**The two candidates** (documents read by the review; nothing run here):
- **LadybugDB on Arrow CSR.** LadybugDB (MIT, C++, Python wheel) mounts relationship tables from Arrow `indptr`/`indices` arrays (`create_arrow_rel_table(..., layout="CSR")`). It reads the arrays that {{#m6-03-arrow-interop}} lends. Risks: the path needed a nightly build in the vendor's notebook; in a symmetric graph mirrored rows count twice (LadybugDB issue #505), and our undirected adjacency stores both directions; the raw CSR costs 8 B per entry with `uint64` indices.
- **Grafeo over a read-only `.eg` store.** Grafeo (Apache-2.0, Rust) has a storage trait (`GraphStore`). A separate crate outside the product implements its required methods over an `.eg` file. No copy of the graph. Risks: the trait is experimental through 0.6.x, so a release may add a required method; one thread per query; a large dependency set (44 workspace dependencies in its manifest).

**Scope**
- [ ] Demand check first: count requests from the feedback loop ({{#m8-09-feedback-loop}})
- [ ] Spike A (about 2 days): LadybugDB in Python mounts the Arrow CSR of lj and orkut and runs 1-hop, 2-hop, `shortestPath` and a triangle pattern. Check zero-copy by the anonymous memory of the process before and after the mount, and the answers against our traversal API
- [ ] Spike B (about 3 days): a Rust crate under `lab/` implements Grafeo's read-only `GraphStore` over an `.eg` file (required methods only) and runs the same queries in GQL and Cypher
- [ ] Choose one engine from the spikes. A path is dropped when it needs a copy of the graph in the engine's own store, or when the adapter passes about 1,500 lines. Success for a path: the four queries correct on orkut, mounted without a copy (A) or without loading the graph into the heap (B), and a 2-hop on a non-hub node under 10 ms
- [ ] Write what the language does not promise: D14 does not hold through an external engine; row order only under `ORDER BY`; no memory budget except our own materialization
- [ ] Note the standards picture: ISO GQL exists since 2024; SQL/PGQ was reverted from PostgreSQL 19 in 2026. Do not chase full conformance
- [ ] LDBC SNB interactive read queries as the benchmark

**Acceptance**
- A written choice of engine and a go/no-go, from the two spikes, before any product code; if go, the SNB short reads run correctly through the bridge

**Not an option.** Our own query engine or planner: parser + DataFusion + our operators.

**Depends on** {{#m8-09-feedback-loop}}, {{#m7-02-shortest-paths}}, {{#m6-03-arrow-interop}} (the Arrow CSR, for spike A)
