---
title: NetworkX backend package
milestone: M9 · After v0.1
labels: type:feature, area:python, P2, risk:medium, size:L, lab-evidence
---
**Why.** NetworkX dispatches to third-party backends; the only thriving one is GPU-only (nx-cugraph, 134 of 797 dispatchable functions). A CPU backend that runs anywhere is an unclaimed adoption channel — with an honest caveat the lab measured: on an existing `nx.Graph`, conversion dominates (lj: 23.4 s to convert; PageRank 2.0× including conversion, 29–38× on the cached graph; components 0.92× including conversion).

**Scope**
- [ ] Separate package (`nx-everygraph`) registered through the `networkx.backends` entry points; conversion cache; correct fallback
- [ ] Native graph objects (`read_edgelist`, `from_pandas_edgelist` equivalents) so users keep NetworkX call sites and skip conversion
- [ ] First 60 functions by usage: centralities, components, shortest paths/BFS, clustering/triangles/k-core, community (Louvain, Leiden, label propagation), degree functions, the conversion utilities the backend test-suite needs
- [ ] Reproduce NetworkX semantics exactly, including the traps found in the lab: PageRank `tol` compared with `N·tol` (stops after one iteration above 2 M nodes), default `weight="weight"`, arbitrary hashable labels (`1 == 1.0 == True`), insertion order of results, generator/set/dict return types
- [ ] Run NetworkX's backend test-suite in CI (lab: 59/59 with fallback, but only 18 calls actually ran in the backend)
- [ ] Documentation that states plainly where the speed-up comes from

**Acceptance**
- NetworkX backend tests pass for the covered functions without fallback
- On a natively loaded lj: PageRank ≥ 25× and components ≥ 20× faster than NetworkX, loading excluded; with loading included, report the ratio (lab: 9.0× and 5.9×)
- No claim of "N× faster NetworkX" without the conversion caveat next to it

**Timing under review (2026-11-25).** Since NetworkX 3.5/3.6, dispatchable readers and constructors can return a backend graph, which may remove the conversion for scripts that load through them. That is the case ADR 0020 named as a reason to change its mind. {{#m1-19-networkx-dispatch}} measures it; if it fires, this issue's milestone is re-decided by ADR (`docs/research/sota-ingest-query.md` C3).

**Estimate from the lab.** 2–3 engineer-months for ~60 functions; ongoing cost tracking NetworkX releases.

**Depends on** {{#m6-01-python-api}}

**Design reference (decided 2026-11-25, `design/DECISIONS.md`).** Deferred to after 0.1 by the approved choice D5 in `design/preview/choices.md`; it stays a separate optional package and is never advertised without the conversion caveat.
