---
title: NetworkX dispatch of readers and constructors: does the conversion cost disappear?
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:python, P2, risk:low, size:S, lab-evidence
---
**Question.** ADR 0020 put the NetworkX backend after 0.1 because on an existing `nx.Graph` the conversion dominates. On lj, 3 threads, shared machine: 23.4 s to convert, PageRank 2.0× and components 0.92× including conversion (FINDINGS § F2, {{#m6-06-networkx-backend}}). Its "what would change our mind" is NetworkX gaining a way to hand a graph to a backend without that cost.

Since NetworkX 3.5/3.6, readers and constructors are dispatchable and can return a backend graph: `read_edgelist`, `parse_edgelist`, `from_pandas_edgelist` and `from_scipy_sparse_array` are `returns_graph=True`, selected through `backend_priority.generators`, and `nx.Graph(backend=…)` through `classes` (3.6). Code that loads through these functions would then never build an `nx.Graph` (`docs/research/sota-ingest-query.md` § 2.5, § 3 D20, C3; NetworkX 3.7 docs and source, read, not run). Does the conversion really disappear in practice?

**Method.** Lab only (`lab/f2-python`), about one day. The run that loads lj is under `systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0`.
- [ ] NetworkX ≥ 3.6 pinned. The lab backend registers `generators` (and `classes` if cheap) so that `nx.read_edgelist(…)` and `nx.from_pandas_edgelist(df)` build a native graph
- [ ] lj from a file: `nx.read_edgelist` then `nx.pagerank`, with and without `backend_priority.generators` set; time from file to result, and peak memory. The same from a pandas DataFrame
- [ ] Which call sites still convert: a script that builds a graph with `G.add_edge` in a loop, or mutates the graph after loading. List them, so the documentation can say which scripts benefit
- [ ] Record the state of the other backends: nx-cugraph is deprecated, RAPIDS 26.12 is its last release (its README)

**Done when.** A table call path × {time file → result, conversion time, peak memory} in FINDINGS, and a statement of whether ADR 0020's "what would change our mind" has fired.

**Decision it feeds.** If loading through the dispatched readers removes the conversion, D20's timing is re-decided by ADR, and with it the M9 placement of {{#m6-06-networkx-backend}}. Moving it before 0.1 also changes an approved design choice (`design/preview/choices.md` D5), so the case is written into `design/DECISIONS.md` first (CLAUDE.md § 8). If it does not, ADR 0020 stands and the result is recorded in it.

**Pointers.** `docs/research/sota-ingest-query.md` § 2.5, § 3 (D20 row), § 4 C3; `docs/adr/0020-networkx-backend.md`; `lab/f2-python`.
