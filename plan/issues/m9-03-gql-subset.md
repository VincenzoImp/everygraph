---
title: Read-only GQL / openCypher subset by reuse
milestone: M9 · After v0.1
labels: type:feature, area:query, P2, risk:high, size:XL
---
**Why.** Analysts coming from Neo4j expect pattern matching. Building a query engine from scratch is years of work and several embedded engines already compete there (LadybugDB, Grafeo, DuckPGQ), so the plan is a bounded, read-only subset built from existing parts — and only if users ask for it.

**Scope**
- [ ] Demand check first: count requests from the feedback loop ({{#m8-09-feedback-loop}})
- [ ] Subset: `MATCH` with 1–3 hop patterns and filters, variable-length paths and `shortestPath`, `WITH`, `OPTIONAL MATCH`, `UNWIND`, aggregation, `ORDER BY`/`LIMIT`, `CALL algo() YIELD`; no writes
- [ ] Reuse: an existing Rust openCypher/GQL parser (candidates noted in the research: `ocg`, Grafeo's GQL parser — verify licences and maturity), DataFusion for relational operators over Arrow, our traversal primitives for expansion
- [ ] Note the standards picture: ISO GQL exists since 2024; SQL/PGQ was reverted from PostgreSQL 19 in 2026 — do not chase full conformance
- [ ] LDBC SNB interactive read queries as the benchmark

**Acceptance**
- A written scope and go/no-go before any implementation; if go, the SNB short reads run correctly

**Depends on** M7
