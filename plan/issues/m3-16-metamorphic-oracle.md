---
title: Oracle: metamorphic relations on the kernels under test
milestone: M3 · Engine and kernels
labels: type:infra, area:algorithms, P1, risk:low, size:M
---
**Why.** The oracle ({{#m0-05-oracle}}, {{#m2-16-oracle-semantics}}) has four families: references, in-process references, invariants and identity across configurations. It has one metamorphic relation (`answers_do_not_depend_on_node_ids`, `crates/everygraph-oracle/tests/properties.rs` l. 107–133), on the *references* only, comparing sorted summaries. Outside the oracle, `crates/everygraph-kernels/tests/metamorphic.rs` (PR #218) checks relabel, disjoint union and induced subgraph (R1–R3) for components, core numbers and degrees, on generated graphs in proptest; the scope below moves them into the plug-in harness and adds the other kernels. Identity across configurations cannot see a bug common to all of them, and references stop at the Python size cap (n > 2^20). Metamorphic relations need no reference, so they also reach graphs above that cap. In the literature they are the productive family on graph code: Graph-cutting (GSlicer, SIGMOD'25) found 39 new bugs in NetworkX, Neo4j and Kùzu, at least 8 logic bugs that baselines including differential testing missed; in Gamera (PVLDB'23) the union and partition relations found 30 of 39 bugs, on 100-node graphs. Decided 2026-11-25 as a fifth oracle family (`docs/research/README.md` T1).

**Scope**
- [ ] `metamorphic.rs` in the oracle: per kernel, relations `G → (G′, f)` with `run(G′) = f(run(G))` exact (bit-identical canonical output) or within a bound; run through the plug-in harness in every configuration (so through the four sources of {{#m3-01-edge-map}}), on the CI corpus and in proptest
- [ ] The relations, from `docs/research/sota-testing.md` challenge 2, first R2/R3, R5, R7 and R11:
  - R1 relabel by a permutation (integers exact; PageRank within its fixed-point tolerance);
  - R2 disjoint union G1 ⊔ G2 (graph-cutting with no cut edges): per-node results equal, totals add; PageRank `(n1/n)·PR_1(v)` when neither part has dangling nodes;
  - R3 induced-subgraph cut G[V1]: core, degree, triangles ≤; BFS/SSSP distances ≥; WCC of G[V1] refines G's;
  - R4 add one edge (triangles ≥, each core number +0 or +1, WCC count −0 or −1, distances ≤);
  - R5 reverse all arcs (SCC equal; BFS `in` on G = `out` on Gᵀ) — reaches the derived in-adjacency ({{#m2-11-in-adjacency}});
  - R6 undirected G against its symmetric directed form (the loop convention A[v][v] = 2w);
  - R7 scale all weights by 2^k: SSSP ×2^k; weighted PageRank, modularity and the deterministic Leiden partition unchanged, **bit-exact** (power-of-two scaling is exact barring overflow and underflow);
  - R8 subdivide every edge (BFS distances ×2 on the original nodes);
  - R9 add a kept self-loop (moved here from the references of {{#m2-16-oracle-semantics}});
  - R10 shuffle rows; split a pair's events (`events`/`sum`, not `collapse`);
  - R11 path decomposition (d(a,c) + d(c,b) = d(a,b) on a returned path) and a zero-weight arc u→v ⇒ d(v) ≤ d(u) — GraphFuzz's zero-weight-cycle class;
  - R12 add a disjoint component or isolated nodes;
  - R13 Leiden: Q of a partition equals Q of its quotient under singletons (exists); relabel gives Q within the documented spread — statistical, not exact
- [ ] Each relation run against the toy kernel and the existing seeded bugs (`toy.rs`), counting which bugs each catches alone, so "caught by ≥ 2 independent families" becomes measurable per relation
- [ ] R1 and R2 on PageRank use the tolerance, not identity, and are never read as a determinism claim

**Acceptance**
- R1–R12 run on the CI corpus in every PR within about 1–2 minutes (the report's estimate; measured here)
- Each seeded bug is caught by at least one relation, recorded per relation in `oracle/README.md`
- The relations run above the Python cap on at least one tier-2 graph in the nightly job

**Feeds.** {{#m0-05-oracle}}, {{#m2-16-oracle-semantics}}, {{#m2-11-in-adjacency}}, {{#m4-01-leiden-production}}, {{#m6-06-networkx-backend}} (the same relations through the public `nx.*` API).

**Pointers.** `docs/research/sota-testing.md` § 2 (Graph-cutting, Gamera, GraphFuzz rows), § 3 (oracle row), § 4 challenge 2; the GSlicer artifact's `oracles.py` (github.com/joyemang33/GSlicer).

**Depends on** {{#m3-01-edge-map}}
