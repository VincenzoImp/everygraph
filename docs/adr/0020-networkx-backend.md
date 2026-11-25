# ADR 0020 — The NetworkX backend is an adoption channel, not the product

- **Status:** accepted
- **Decision:** D20 in [DESIGN.md](../DESIGN.md#4-decisions)
- **Date:** 2026-11-25

## Context

NetworkX can dispatch to backends with no code change by the user. A backend is a cheap
way to reach its users, but it starts from NetworkX's own graph objects.

## Decision

A NetworkX backend is built as an adoption channel. The product is Everygraph's own API
with native loading. The backend was moved after v0.1 by an approved design choice
(`design/DECISIONS.md`, 2026-11-25).

## Evidence

From the backend prototype (FINDINGS § F2; shared machine; lj; 3 Rayon threads;
`lab/f2-python/results/goalC_lj_dev.json`):
- **On an existing `nx.Graph`, conversion dominates**: converting took 23.4 s. PageRank
  (tol 1e-10) 61.5 s in NetworkX vs 30.4 s through the backend including conversion
  (2.0×); components 14.5 s vs 15.8 s including conversion (0.92× — slower). DESIGN's
  "1–3×" is 0.92–2.0× in the result file.
- **On a cached converted graph**: PageRank 29–38× (2.09 s and 1.60 s); components kernel
  alone 121× (0.119 s). FINDINGS' "121–177×" upper bound is **not traced**.
- **With native loading** (graph read by Everygraph, not converted): PageRank 15.6×
  (3.93 s, tol 1e-10) and 70× (0.72 s, default tol); components 24× (0.60 s, labels as a
  list) and 53× (0.27 s, count only). DESIGN's "100×+ only with native loading" is **not
  supported** by this file; above 100× appears only for the components kernel alone.
- Semantic traps found: PageRank `tol` scaled by N, default `weight="weight"`, arbitrary
  hashable labels, insertion order. Credible coverage needs ~100–150 of 797 functions.

## Consequences

- No "N× faster NetworkX" claim appears without the conversion caveat next to it (#86's
  acceptance).
- The public speed story uses native loading through Everygraph's API (#81).
- #86's target (natively loaded lj: PageRank ≥ 25×, components ≥ 20× faster "end to end")
  is consistent with the measured kernel ratios only if loading is excluded. With loading
  included (NetworkX builds from CSV in 51.5 s, the native reader in 10.6 s), the same file
  gives 9.0× for PageRank at default tolerance (11.3 vs 101.9 s) and 5.9× for components
  (11.2 vs 66.0 s).

## What would change our mind

- NetworkX gains a zero-copy way to hand a graph to a backend, removing the conversion
  cost; the backend could then carry the product's speed story.
