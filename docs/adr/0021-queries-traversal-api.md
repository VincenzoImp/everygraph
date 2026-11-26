# ADR 0021 — Queries in v0.1: a traversal API with budgets, no query language

- **Status:** accepted, partly validated
- **Decision:** D21 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

Users ask point questions (neighbours, k-hop, paths) as well as whole-graph analytics.
Embedded graph databases offer Cypher or GQL; building a query language is large, and
the engine's strength is the in-process kernel.

## Decision

Version 0.1 offers a traversal API with budgets (`limit`, `timeout`) and no query
language. A GQL/Cypher subset comes later (#110), through one embedded engine that reads
our graphs: LadybugDB on the Arrow CSR that #83 lends, or Grafeo over a read-only `.eg`
store. We do not build our own query engine (amended 2026-11-26, from the state-of-the-art
review, `docs/research/sota-review-2026-11.md` S13).

## Evidence

- **Our prototype** (FINDINGS § F1; shared machine; mmapped *uncompressed* CSR, warm,
  single thread, p50; `lab/f1-query/results/pq_lj_warm.json`,
  `lab/f1-query/results/pq_orkut_warm.json`, `lab/f1-query/results/pq_twitter-dir_warm.json`):
  degree/neighbours 0.1–0.6 µs; 3-hop with a 10k budget 56–115 µs; bidirectional BFS
  0.3 ms (lj), 0.6 ms (orkut), 1.5 ms (twitter-2010). p99 tails on hubs up to 15 ms
  (2-hop) and 113 ms (path). Cold on Friendster, a shortest path took 0.8 s.
- **Other engines through their query interfaces** (FINDINGS § F2; shared; lj, 3 threads,
  warm; `lab/f2-python/results/goalA_summary.txt`): shortest path Neo4j 6.4 ms, igraph
  291 ms, LadybugDB 2,550 ms, rustworkx 4,785 ms; DuckPGQ ran out of memory above 6 GB.
- **Not like for like**: an in-process kernel on raw arrays against calls through a query
  interface, at different thread counts — different measurement levels (CLAUDE.md § 3).
  It shows where the cost lies, not a speed-up to quote.
- **Reuse is possible**: Rust parsers exist (`ocg`, Grafeo's GQL parser); SQL/PGQ was
  reverted from PostgreSQL 19 (FINDINGS R8, web research).
- **Whole engines can run on our data** (review of 2026-11-26, documents read, nothing
  run): LadybugDB mounts relationship tables from Arrow `indptr`/`indices` arrays, and
  Grafeo has a storage trait that a read-only `.eg` adapter can implement. Either gives a
  query language without a planner of our own. The LadybugDB and DuckPGQ figures above are
  from older versions and are to be re-run (FINDINGS § F2).
- **Idle, on the compressed default** (#118, 2026-11-25; FINDINGS § M1-13 and its
  correction of § F1; `lab/m1-13-compressed/results/pq.jsonl`; in-process, single thread,
  warm, graph in heap): bidirectional BFS p50 0.13 ms (lj) and 0.47 ms (twitter-2010) on raw
  arrays, 0.16 and 0.56 ms on the default storage; across lj, orkut, twitter-sym, Friendster
  and twitter-2010 the default costs 1.13–1.41× at p50 for bidirectional BFS and 2.5–4.2×
  (+0.02–0.11 µs per call, timer-free batch means) for neighbour lookups. The
  shared-machine figures above are 1.5–2.8× pessimistic. Point-query throughput at
  16 threads on the default is measured only exploratory (the pull request of #90; lj and
  orkut, loaded machine): neighbours 2.1–2.5× slower than raw on orkut without the decode
  tier, 3-hop 1.0–1.2×; the idle-machine run is #m7-xx-query-performance's.

## Consequences

- #90 (point queries) and #91 (shortest paths) build the API; `limit` is the word for
  traversal budgets, "budget" is kept for memory (`design/DECISIONS.md`).
- No query-language parsing before v0.1; #110 is in M9.
- #83 lends the structure as Arrow `indptr`/`indices` arrays; #110 runs two short spikes
  (LadybugDB on Arrow CSR, Grafeo over `.eg`) and chooses one. Results through an external
  engine are outside D14, and the docs say so.

## What would change our mind

- #118 shows bidirectional BFS or neighbours on the default format more than 5× slower
  than raw, so that #91's p50 < 2 ms on twitter-2010 needs a resident raw copy. (Not
  triggered on 2026-11-25: 1.13–1.41× and 2.5–4.2×, single thread; see Evidence.)
- Feedback collected after release (#106) shows that most target users cannot express
  their questions with the traversal API.

## Closed by

#91 (shortest paths at 10^9 arcs).
