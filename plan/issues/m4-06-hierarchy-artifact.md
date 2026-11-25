---
title: Hierarchy artifact with exact aggregates
milestone: M4 · Communities and hierarchy
labels: type:feature, area:community, P0, risk:low, size:M, lab-evidence
---
**Why.** The map, the per-community tables and the "exact numbers at every level" promise all read this artifact. The lab format was verified independently (node counts sum to n, volumes to 2m, internal + cut edges to m, quotient row sums equal volumes) and 1,403,208 lj communities matched between two independent implementations.

**Scope**
- [ ] Per level k: parent map, node count, degree volume, internal edge count (or weight), cut weight; quotient graph in the standard adjacency format with integer weights (edge counts, exact) for unweighted inputs and f32 for weighted ones unless exactness requires f64 (changed 2026-11-25 from "f64 weights": the tracer's map-shaped Friendster hierarchy is 42.5 GB, 2.7× the 15.5 GB CSR, about two thirds of it f64 quotient weights by arithmetic on the per-pass entry counts; {{#m1-01-e2e-tracer}}, `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 6 and 14, `lab/m1-01-tracer/results/b2_louvain_capped_56G.json`)
- [ ] Each level's quotient graph is streamed to disk as community detection produces it, never all levels kept in memory, and written without a whole-level conversion copy of its weights. In the lab, `--out-hier` keeps every level (the capped Leiden was killed at 56 GiB, about 51.5 GiB of quotient graphs by arithmetic), and the capped Louvain's 42.9 GiB anonymous peak against b2's own 28.3 GiB before the write is most likely the writer's transient f64 copy of the level-1 weights (2.14 B entries, about 16 GiB as f64; inference from code and numbers by the review of the tracer's lab PR; `bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` rows 10, 13–14)
- [ ] Levels streamed to disk as compressed stores, as measured by {{#m1-02-ooc-aggregation}} (2026-11-25, idle, PR #143, FINDINGS § M1-02): with `--out-hier`, every quotient graph is written as a weighted compressed store and memory-mapped, never held in memory; map-shaped Leiden (cap 64, compaction 3) on Friendster then completes under `MemoryMax=16G` in 164.1 s, anonymous peak 10.9 GiB, Q 0.5900, 27 top communities (`bench/results/ooc-leiden/20261125T160906Z-e501e2cf5727.jsonl`, case `ooc-friendster-map/limit=16G/t16`), where the tracer's b2 was killed at 56 GiB. On orkut the map-shaped level files are byte-identical in RAM and out of core (`lab/m1-02-ooc-leiden/results/map_identity.jsonl`, exploratory). Gaps for this issue: the out-of-core hierarchy writes level maps and quotient stores but no per-level volume (`level_k.volume.u64`) or internal-edge (`internal_edges.u64`) files, which the in-RAM output has — this artifact must write them on the streamed path too (review of #143); and its `<out>.quot.<pid>/` directory is left behind when the run is killed
- [ ] Stored as a derived artifact with provenance ({{#m2-12-artifact-cache}})
- [ ] Conservation checks run on write and in `everygraph verify`
- [ ] Each level's quotient graph carries a loop at every community of weight L(c), the input's kept loops included, so its total weight is W and its modularity under singletons equals that of the partition; `internal_edges` counts a kept loop's events (`docs/spec/graph-semantics.md` § 5.5, S6)
- [ ] API: membership at any level as a column, community table as Arrow, `subgraph(community)` as a view, ancestors/descendants of a community
- [ ] Region ids are stable: a deterministic run (D14) gives every community at every level the same id on every run of the same input and seed, so a region can be cited by id — by the map, by the MCP server's region card ({{#m9-02-mcp-server}}) and in a user's notes (added 2026-11-25, `docs/research/README.md` H8)
- [ ] Per-community summary statistics of node properties (counts by category, top-k by PageRank) for map labels

**Acceptance**
- Invariants hold on every tier-1/2 graph and every level
- On every oracle graph with kept loops, each level's modularity equals the modularity of the level below under the partition it represents (`docs/spec/graph-semantics.md` § 5.5)
- Reading a community table of 1 M communities takes < 50 ms
- Friendster map hierarchy: written with memory that does not grow with the number of levels, and its size on disk reported against the tracer's 42.5 GB (`bench/results/tracer/20261125T122609Z-3e764d115d86.jsonl` row 14)

**Pointers.** `--out-hier` in `lab/b2-leiden`; `/data/hier` format in LAB.md.
