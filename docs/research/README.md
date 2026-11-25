# Research reports

Literature reviews written for Everygraph. They are inputs to the plan, not decisions: a
report says where the literature puts us (behind, level, ahead) and proposes the cheapest
experiment that would settle each doubt; the plan (`plan/issues/`) and the ADRs decide.
Every number in a report is the paper's own, on the paper's machine, and is not comparable
with ours as it stands. Nothing in these reports was built or measured.

## The reports

| Report | Area | What it covers | Read depth |
|---|---|---|---|
| [memory-bounded-algorithms.md](memory-bounded-algorithms.md) | algorithms under a memory limit | per kernel class (communities, components, BFS, k-core, triangles, centrality, PageRank, layout, out-of-core systems); ideas for out-of-core Leiden level 0; an addendum (2026-11-25) verifying a second list of 18 papers and 4 resources (Seraph, Grafu, Gem, Planar, LDBC Graphalytics, Web Data Commons, …) | primary pages opened; many items abstract or snippet only, marked as such |
| [sota-storage.md](sota-storage.md) | storage | compressed adjacency codecs (Stream VByte, Ligra+, BV/Zuckerli, Elias–Fano), vertex orderings (BP, LLP, Rabbit, Gorder, DBG, BOBA), D3–D6, m1-09, m1-14, m2-07, m2-20 | about 14 works in full, the rest partial or abstract |
| [sota-io-memory.md](sota-io-memory.md) | I/O and memory | mmap vs buffer pools (Crotty, vmcache, TriCache, uCache), NVMe, cgroup accounting, allocators, external sort and spill; D2, D11, the accountant, m2-19, the safe-mmap row | 4 works in full by the author, about 30 in full by helpers with numbers spot-checked |
| [sota-kernels.md](sota-kernels.md) | parallel kernels | GBBS, Ligra(+), GAP, ConnectIt, BGSS SCC, ρ-/Δ*-stepping, Liu et al. k-core, LOTUS, propagation blocking, deterministic parallelism (Mt-KaHyPar-SDet); D12–D14, #44, #47–#52 | mostly full (delegated), key numbers checked against the text |
| [sota-communities.md](sota-communities.md) | community detection and map hierarchies | Louvain/Leiden variants (GVE, NetworKit PLM, DSLM, pLeiden, CluStRE), sub-round determinism, prolongation refinement, recursive sub-clustering (ASK-GraphView, graspologic, hierarchical Infomap); D14–D16, #18, #19, #57, #63, m4-09 | about 15 works in full, the rest results or abstract |
| [sota-maps.md](sota-maps.md) | layout, level of detail, tiles | GMap, Uncharted (Langevin 2015), ASK-GraphView, sfdp/FM³/DRGraph/SNAP-tFDP/BatchLayout, Voronoi treemaps, Muelder–Ma, edge clutter and matrices; D16–D18, D25, #19–#21, #67, #69 | about 10 works in full, about 20 full (delegated), the rest abstract |
| [sota-ingest-query.md](sota-ingest-query.md) | ingest, ids, format, views, API | MPHFs (PtrHash, PHastR), parsing (csveee, simdjson), CSR build and sorting (PIGO, IPS4o), containers (GraphAr, Lance, Arrow IPC, BtrBlocks), engines (Kùzu, DuckPGQ, Neo4j GDS, NetworKit), NetworkX dispatch; D7–D10, D19–D21, ADR 0026 | mixed: full, partial and docs, by four readers |
| [sota-coverage.md](sota-coverage.md) | coverage audit | which works and topics the six area reviews missed; the scopes of the second wave | abstracts only, by design |

**A second wave is in progress** (scoped in `sota-coverage.md` § (d)): bit-identical
numerics, testing a graph engine, honest measurement, dynamic graphs; and, as follow-ups,
human factors of graph maps and approximation / Rust practice. Its reports will be listed
here when they land.

## Decisions taken from the first wave

Decided 2026-11-25 by the coordinator after the first wave. Rule: nothing below changes a
D-number decision without evidence. Where the literature says we may be wrong, the change
is an experiment issue whose result decides (then an ADR). Text that was simply inaccurate
was corrected at once. Where each item landed is in the pull request that applied them
("Plan and docs: apply the first research wave").

### A. Corrections (text only)

- **A1. Novelty claims** (README, DESIGN, FINDINGS). Prior art: a hierarchical map with
  data tiles (Uncharted / Aperture Tiles, Langevin et al. 2015; [maps § 5](sota-maps.md)); a
  fan-out cap of 64 (ASK-GraphView 2006); compressed adjacency faster than raw in parallel
  (Ligra+ 2015; [storage § 5](sota-storage.md), [kernels § 5](sota-kernels.md)); recursive
  sub-clustering (ASK-GraphView, hierarchical Infomap, graspologic;
  [communities § 5](sota-communities.md)); Elias–Fano offsets (webgraph-rs); a seeded
  relabelling of node order (DSLM 2018); planning and refusing before a run (Neo4j GDS
  memory estimation; [ingest-query § 5](sota-ingest-query.md)). What we can claim:
  identical results across thread count, budget, regime and view representation; a
  deterministic, budgeted text → CSR build; a 1.8 B-edge map on one machine under a budget,
  with byte identity and declared omissions; one seeded permutation for storage, layout and
  visiting order; the planner's raw decode-cache tier with its break-even rule; refusal
  before work where DuckDB and Velox fail mid-run ([io-memory § 5](sota-io-memory.md)). The
  super-level rule is a heuristic (no prior art found).
- **A2. DESIGN § 6, Friendster quality target.** "Q ≥ 0.60" is not a quality claim: plain
  Louvain, NetworKit PLM and Grappolo publish 0.622–0.627 ([communities § 3](sota-communities.md),
  DSLM Table 2). New target: the quality mode within 1% of the best of NetworKit PLM and
  GVE-Leiden run on this machine (same graph, same preprocessing), with the fast mode's gap
  stated; three iterations on Friendster must be measured (#57).
- **A3. The 1%-of-best gate on lj** names its reference: igraph Leiden 0.7695 (one run, this
  machine); Traag et al.'s 0.7739 is a best of 10 and not like for like.
- **A4. #114 (u64 ids).** The Web Data Commons page graph has 3.56 B nodes, below 2^32.
  Validation needs a synthetic RMAT graph above 2^32 nodes (Seraph used 8.6 B) or Software
  Heritage. The WDC host graph (101 M nodes, 2.0 B arcs) goes to #24 as a web-family graph
  ([memory-bounded addendum, item 18](memory-bounded-algorithms.md)).
- **A5. m4-09, label propagation.** CluStRE numbers corrected (0.5871 is the Strong mode;
  Light+ 0.5839; its 10-minute wall-clock cut-off makes it non-deterministic); not a
  fallback. Decided: LPA follows LDBC CDLP semantics (initial label = the node's original key
  order, ties → smallest label), so results validate against LDBC reference outputs; a
  relabelling (#146) must keep id-dependent semantics on original ids.
- **A6. D9 text:** the edge id is u32 or u64 per section (m2-10 already says so).
- **A7. `docs/spec/eg-adjacency.md` / D7:** encoders are deterministic functions of their
  input (no randomly sampled codec choices, unlike BtrBlocks).
- **A8. `design/DECISIONS.md`, safe mmap:** map only files on local block devices; an I/O
  error under a map is SIGBUS where `pread` returns EIO ([io-memory § 3](sota-io-memory.md)).
- **A9.** The earlier survey's blanket "asynchronous = no" (§ G.2, § I.9 of
  memory-bounded-algorithms) narrows to order-sensitive kernels (PageRank sums, Leiden,
  LPA). Monotone fixpoint kernels (min-label WCC, BFS depth, SSSP, k-core) have a unique
  answer: order-free execution and skipping inactive lists are allowed for them.
- **A10. m3-05:** SCC uses BGSS (Wang et al., SIGMOD'23), not forward-backward with
  trimming; WCC uses ConnectIt UF-Rem-CAS with hybrid k-out sampling (first + one seeded
  random neighbour), not first-neighbour sampling, which fails on locally clustered orders
  ([kernels § 3–4](sota-kernels.md)).
- **A11. m3-07 k-core:** designed after Liu et al. (SIGMOD'25): active set, sampled hub
  degrees (still exact), vertical granularity control, hierarchical buckets; "cap threads"
  dropped as the fix.
- **A12. m3-06 SSSP:** Δ*- or ρ-stepping with Δ chosen from sampled weights by a
  thread-count-independent rule, plus bucket fusion; a fixed Δ is fragile (up to 4.92×).
- **A13. m3-04 PageRank:** f64 is required for LDBC-comparable runs (#100); f32 stays an
  option.
- **A14. M3 exit criterion:** "faster than NetworKit" becomes "at least GAP reference speed
  on this machine, reported against GBBS" (#55 runs GAP, GBBS, ConnectIt and Liu's k-core
  here).
- **A15. m2-07 / #34:** the default id order is natural (sorted integers, first appearance
  otherwise), recorded as an acceptance criterion; never hash order.

### B. Experiments (new or widened issues), each with its cheapest test

- **B1.** mmap hints, then a budgeted block cache over `ByteSource`
  ([io-memory C5, C1](sota-io-memory.md)); if it wins, an ADR amends D11 (regime 2 = an
  explicit cache for random access, mmap for warm data and scans).
- **B2.** An ordering bake-off (m1-09 widened; [storage C1, C5, C7](sota-storage.md)); #146
  reframed (random relabelling is the literature's worst case); a deterministic BP ordering
  as an opt-in compaction step, never inside promise 1's ingest.
- **B3.** Block codec 2, Stream VByte 0124: offline word-length histograms first; m2-20 waits
  for B2 and B3 and is judged against BV.
- **B4.** Chunked hub records ([storage C3](sota-storage.md), [kernels C4](sota-kernels.md)).
- **B5.** Deterministic Leiden sub-rounds at {4, 8, 16, 32, 64, auto} (#18); D14's cost is
  re-measured after, not narrowed before.
- **B6.** Leiden quality (#57): prolongation refinement, a refinement faithful to Traag
  2019, split every pass vs last, against same-machine NetworKit PLM / GVE-Leiden.
- **B7.** CSR build without spill when the targets array fits; hybrid buckets under 8 GB;
  exact bucket boundaries; #35's scaling target corrected to the ~8× sort ceiling.
- **B8.** Large buffers never through `malloc`; `mallopt` pins; mimalloc as an A/B (m2-19).
- **B9.** The ledger reconciled with a cgroup monitor (m3-10, m2-19) and a planner rule (m3-02).
- **B10.** Map layout: stress placement of siblings, power-diagram cells, a published-layout
  baseline (new M1 issue), hierarchy-order I/O and space-filling curves; D17 re-decided after
  the baseline.
- **B11.** An overview without selection (matrix, external-weight glyph, "showing X% of
  inter-region weight"): it changes the approved design, so a preview comes first.
- **B12.** LDBC Graphalytics as the large-graph oracle and a benchmark workload.
- **B13.** Propagation-blocked PageRank; first, DRAM bytes per entry of the current pull kernel.
- **B14.** Monotone kernels out of core: alternating scan direction, decoding only active
  lists, bound pruning.
- **B15.** NetworkX dispatch: a one-day prototype; if conversion disappears, D20's timing is
  re-decided (ADR 0020's "what would change our mind").
- **B16.** A staged id-mapping loop.
- **B17.** Arrow IPC framing for column sections.
- **B18.** Views that skip work: node masks over list offsets, block zone maps.
- **B19.** Per-region map jobs and labelled binning (follow-ups of PR #162).
- **B20.** API gaps for v0.1: a one-call graph profile, node tables, a public edge-type
  name, a batch of path pairs.

### C. Not adopted

Kept in the reports, one line each: PHastR MPHF (about 1% of the dictionary; `unsafe`
dependencies) · GraphAr, Lance or Vortex as the container · userfaultfd, UMap, FastMap,
exmap, SPDK · SSDAlloc, AIFM, OMRGx (transparent far memory) · RStream, Kaleido (pattern
mining) · Rabbit Order (non-deterministic) · ACGraph / Grafu-style asynchrony for
order-sensitive kernels · Demmel–Nguyen order-free summation for PageRank (3–7×; fixed
reduction blocks kept), to be revisited by the second wave's numerics review for weighted
Leiden.
