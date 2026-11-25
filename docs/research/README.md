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

**The second wave** (scoped in `sota-coverage.md` § (d)): six reports, turned into
decisions in section D below.

| Report | Area | What it covers | Read depth |
|---|---|---|---|
| [sota-numerics.md](sota-numerics.md) | bit-identical numerics | reproducible summation (Demmel–Nguyen, ReproBLAS, Hallberg–Adcroft fixed point, Müller in databases), FMA and contraction in Rust/LLVM, libm and correct rounding, counter-based RNG (Philox), wasm NaN bits and relaxed SIMD, order-revealing tests (FPRev); a rule list R1–R15 for the determinism CI | full for the core set; three papers not accessible (HAL anti-bot); one compile-only check of rustc's output |
| [sota-testing.md](sota-testing.md) | testing a graph engine | differential and metamorphic testing (GraphFuzz, Graph-cutting/GSlicer, Gamera), property-based generation, format and resource-exhaustion fuzzing (MemLock, PerfFuzz, Singularity), decompression bombs, crash consistency (ALICE, CrashMonkey, CORDS, fsync failures), mutation testing | full by the author or by helpers, checked; GSlicer through its poster and artifact (ACM 403) |
| [sota-measurement.md](sota-measurement.md) | honest measurement | repetition and confidence intervals, measurement bias, interleaving, CPU and drive effects (the lab drive's fast-write burst, TRIM), graph-benchmark pitfalls, how to publish comparisons | full by the author or by helpers, checked; two official result files re-analysed offline |
| [sota-dynamic.md](sota-dynamic.md) | graphs that change | dynamic storage (LSM-of-CSR, PaC-trees, Sortledton), snapshots, incremental kernels and Leiden, map stability after an append; what `.eg` must reserve now | full, partial and abstract, marked per work |
| [sota-human-factors.md](sota-human-factors.md) | human factors | accessibility of graph views, API usability, privacy of shared maps, graphs for LLM agents | full text for a core set; several key papers abstract only (bot walls), stated in the report |
| [sota-approx-rust.md](sota-approx-rust.md) | approximation and Rust practice | sketching and approximation under a memory budget (HyperBall, sampling for centralities), Rust parallel practice (Rayon, `unsafe`, fearless parallelism) | full, method-and-evaluation or abstract, marked per work |

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
  ([io-memory C5, C1](sota-io-memory.md); the report's "m2-21" is filed as m2-23, the
  number being taken); if it wins, an ADR amends D11 (regime 2 = an
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

## D. Decisions taken from the second wave

Decided 2026-11-25 by the coordinator after the second wave. The rule of the first wave
holds: text that was wrong is corrected now; a change of a D-number decision needs an
experiment issue first and an ADR after. The item labels below (N, T, M, D, H, R) are this
section's own and are not DESIGN's D-numbers. Already acted on elsewhere and not repeated
here: the event-model defect of `docs/research/sota-dynamic.md` § 0.1 (PR #184, ADR 0028);
the container reservations, float storage and crash-model additions (PR #170); the inputs
to the edge-map framework (#44, sent to its author). Where each item landed is in the pull
request that applied them ("Plan and docs: apply the second research wave"). Every number
below is the paper's own, on the paper's machine, unless it names one of our result files.

### Numerics ([sota-numerics.md](sota-numerics.md))

- **N1. Fixed-point i64 for every sum whose order depends on the schedule** — in Leiden
  (σ, aggregation, the gain bookkeeping) and in map aggregates. The scale 2^s is chosen once
  from the exact total weight T so that T·2^s ≤ 2^62; unweighted and integer-weighted graphs
  keep today's bits. This lifts m4-04's integer-weight-only restriction. Fixed order is the
  wrong pick: it constrains every future aggregation and the atomic commit, and one
  optimization breaks it silently; integer sums cost what float sums cost (Müller et al.,
  Fig. 4), binned accumulators 1.9–2.4× and 48 B each ([numerics § 3.3](sota-numerics.md)).
  Into m4-02 (the scheme; its "modularity equal across aggregation" needs exact sums), m4-03
  (the cost A/B: Friendster level-1 aggregation, f64 against i64 fixed point) and m4-04.
- **N2. The rule list R1–R15** of [numerics § 4](sota-numerics.md) goes into m3-11,
  enforced mainly by clippy `disallowed-methods`/`disallowed-types` in the shared lint
  (m2-18) and by `deny.toml`: std transcendentals and `powi`/`powf` in result paths,
  fast-math and relaxed variants, randomly seeded hash maps whose order reaches a result,
  entropy-seeded RNGs, `Iterator::sum` on floats, `f64::max/min` where signed zeros can tie.
- **N3. Determinism CI (m3-11):** weighted test graphs (sums of integers are exact in any
  order, so a reduction-order bug passes on unweighted graphs today) and FPRev-style order
  probes; cross-platform hash tests native ↔ wasm32 and x86-64 ↔ aarch64 (qemu) for
  PageRank, Leiden and the layout. ADR 0014's platform evidence (PageRank compared by sum and
  maximum, not by a hash; Leiden on x86-64 only) is weak, and m3-11 says so.
- **N4. One pinned pure-Rust math module** with golden vectors checked on x86-64, aarch64,
  i686 and wasm32; it replaces std transcendentals wherever they reach a result (lab layout
  `cos`/`sin`, tile zoom `log2`, Leiden's tolerance `0.1f64.powi(iter)`, the oracle
  generator's `powf`). New issue m2-26; it feeds m3-11, m4-01, m5-01, m5-03.
- **N5. A counter-based RNG in the core** (Philox4×32-10) with a written key schema (seed,
  purpose, entity ids, round, draw); thread id, chunk, budget and any sequential stream never
  enter the key; `thread_rng`, `StdRng`, `SmallRng`, `fastrand`, `getrandom` and `usize`
  sampling are banned in result paths. New issue m2-27; R1 and R2 below use it.
- **N6.** Into m2-26 or m3-11: an exact accumulator for global scalars (the class of the
  PR #131 bug); canonical NaN and no −0.0 in results; an FTZ/DAZ guard at entry from the
  Python module; `relaxed-simd` a compile error in wasm builds; and the supported-platform
  list (x86-64, aarch64, i686 with SSE2, wasm32 without relaxed SIMD; i586/x87 and 32-bit ARM
  NEON floats excluded, Rust RFC 3514) written into D14's text as a list of platforms only —
  the promise is not narrowed anywhere else.
- **N7. Novelty:** identity across memory budgets and execution regimes is new for graph
  engines as far as read, but the concept exists elsewhere: decomposition-invariant sums in
  HPC (Hallberg–Adcroft 2014) and data independence in databases (Müller et al. 2018). The
  README and DESIGN novelty sections say so.

### Testing ([sota-testing.md](sota-testing.md))

- **T1. Metamorphic relations on the kernel under test** as a fifth oracle family: relabel,
  disjoint union, induced-subgraph cut (Graph-cutting), arc reversal, weight scaling by 2^k
  (bit-exact, deterministic Leiden included), path decomposition, zero-weight arcs. New
  issue m3-16, about 1–2 minutes of CI (the report's estimate).
- **T2. Budget fuzzing (m3-10):** a growth-slope test of untracked heap (counting-allocator
  peak minus ledger peak) over n = 2^10…2^16, where any bytes-per-node slope fails; a search
  over generator parameters (MemLock, targeted PBT); PerfFuzz-style per-phase maxima; a
  binary search of the true minimum budget against the planner's declared one; failure of the
  k-th reservation; the cgroup matrix becomes nightly. The motivating case is #34's
  `build-500m` case, OOM-killed under `MemoryMax=5G` with a 4 GiB budget in the official run
  `bench/results/id-dictionary/20261125T220117Z-c855fdd77cf9.jsonl`.
- **T3. One fault-injecting `ByteSource`/`ByteSink`** (ENOSPC or EIO at the k-th operation,
  short reads, the states after a failed sync), swept over every k; a real ENOSPC tried
  through a user namespace and a size-limited tmpfs. New issue m2-29.
- **T4. Fuzzing in CI** (new issue m2-30): replay of the corpora on every PR, 5-minute
  campaigns when a target's crate changes, nightly long runs with canary bugs; new targets
  for the container and the text reader. **cargo-mutants** gets its own issue (m3-17):
  `--in-diff`, sharded, advisory on PRs; reserve/capacity mutants are kept, not treated as
  arid; a mutation score is never a correctness argument (Just et al.: 17% of 357 real faults
  are coupled to no mutant).
- **T5. m6-06:** parity with NetworkX's exceptions, and degenerate graphs (18 of GSlicer's
  25 NetworkX bugs).
- **T6.** m2-06 (Parquet) and m5-02 (tiles) gain hostile-input and declared-size items; a
  regression test pins that ruzstd decoders are not reused across frames (a reused decoder
  pre-reserves the frame's declared window, up to 128 MiB, outside the ledger) — in m2-30.

### Measurement ([sota-measurement.md](sota-measurement.md))

- **M1. Drive state against drive** (experiment m1-21; it can change ADR 0026, promise 1
  and #35): drive state in every harness row (firmware, fill, discards, time since the last
  fstrim, NVMe temperature); PR 136's 64 GB write probe re-run right after the next fstrim
  (Mon 2026-11-30 01:08) and again 48 h later; then promise 1 restated as bytes written per
  edge × the drive's rate, by an ADR amending 0026. The "~10 GB burst" is probably host
  state, not the drive model (a review of the same model empty reports ~240 GB); m2-08
  notes it.
- **M2. Harness statistics** (m3-18, and m3-12's gate): the interval printed with its
  confidence level; 6 official repetitions by default (with 3, [min, max] is a 75% interval
  of the median); runs compared by a ratio interval against a per-case threshold. The
  planned 7% gate would fire about three times per suite with no code change: 3 of 68
  headline cases already differ by ≥ 7% between two idle runs of the same revision.
- **M3. `--interleave`:** randomized interleaved rounds in the harness (m3-19). CLAUDE.md
  § 6 already requires interleaved A/B runs; the harness cannot do them yet.
- **M4. A public comparison protocol** (#100 / m8-03): the same input ordering for every
  tool; isolated vertices stated; BFS from 64 seeded non-zero-degree sources; PageRank in f64
  with a tolerance; load, kernel and end to end reported separately; a single-thread COST
  line; Graphalytics validation; randomized rounds with ratio intervals.
- **M5. Planner (m3-02):** weight writes 2.4–2.8× reads when sustained (Viglas' rule, λ =
  read rate ÷ sustained write rate on the lab drive); wear does not justify recompute over
  spill (≈ 0.008% of rated TBW per Friendster pipeline, arithmetic).
- **M6. Robustness of evidence:** D4's ratios near 1.0, D13's "SMT gives nothing" and D11's
  "~10%" need ratio intervals from interleaved runs before they are restated. Added to the
  issues that will re-measure them (m1-08, m3-04), not to DESIGN now.
- **M7. Laptop runs in m8-03:** a cool start and a back-to-back steady state.

### Dynamic graphs ([sota-dynamic.md](sota-dynamic.md), beyond § 0)

- **D1. "Merging or compacting segments never changes a result"** gets its own issue before
  any m9-04 code (m9-11): neighbours in canonical merged order or order-independent sums,
  reduction blocks fixed by node count, pair sums over all segments; and a test.
- **D2. m9-04:** levelled, compressed, immutable segments; a sparse segment index; merge
  factor about 10 (LSMGraph). That the edge-map framework must accept a merging source was
  told to #44.
- **D3. m7-05:** "as of" questions answered in one time-ordered pass (union-find over 100
  time points).
- **D4.** Incremental Leiden and stable map updates are deterministic with synchronous rounds
  (LD-Leiden) and exact hierarchy repair, but their results depend on the batch sequence;
  D14's "same input" means the same base and the same sequence of batches (the sentence of
  PR #184's DECISIONS row). D17: a stable map update is a function of the new graph and the
  previous map file. Into m9-04 as later work.

### Human factors ([sota-human-factors.md](sota-human-factors.md))

- **H1. `reveal="summary"` is specified before the tile format freezes** (m5-02, m5-09): no
  node-level entities, per-node adjacency, search index or node attributes; regions under k
  merged into `aggregate`; inter-region aggregates below k merged into "other" with
  consistent totals; provenance reduced to a label; attribute summaries and the time slider
  off unless asked; k in the metadata and the caption. New issue m5-15, "Summary export:
  k-threshold merging, stripped provenance, a reconstruction test", in v0.1.
- **H2. Non-visual navigation of the map hierarchy**, split from m5-11 (new issue m5-16):
  a structure layer that renders only the focused region, `aria-setsize`/`aria-posinset`,
  structural and targeted keys, a description template, children as a table, the focused
  region highlighted on the canvas; a formative session with 3–5 screen-reader users.
- **H3. The keyboard model is decided:** arrows move through the structure (as
  `foundations.html` § 9 says), Shift+arrows pan; `/ + - 0` act only while the map has focus;
  Esc in the Jupyter widget never traps the keyboard (WCAG 2.1.2). A `design/DECISIONS.md`
  row records it: it resolves a contradiction between two approved previews in favour of the
  written rule. m5-06 and m5-10 reference it; `viewer.html` is updated when m5-06 is built.
- **H4. Usability evidence:** sessions of 5 people are formative; a success rate is claimed
  only from n ≥ 20 per task with an exact interval (4 of 5 has an exact 95% lower bound of
  0.28); "usable with a screen reader" only after a session with screen-reader users; an
  LLM-in-the-loop check of the docs in m8-02 as a regression test, not as evidence. Into
  m5-06, m6-08, m8-08, m8-02.
- **H5. WCAG 2.2 AA checklist in CI** (m5-11): 1.4.11, 2.5.7, 1.4.10, 4.1.3, 2.3.3, and
  200% zoom.
- **H6. Command line:** the piped form of progress also under `TERM=dumb`, plus
  `--progress=plain` (m8-01; `docs/ux-contract.md` § 2.2).
- **H7. m8-09:** never "anonymized"; a notice that structure alone can identify people.
- **H8. m9-02 (MCP):** labels and properties are marked as untrusted data; the reveal level
  applies to agents; a "region card" and a top-down "drill" tool; region ids are stable
  (m4-06 says so).
- **H9.** A research issue "DP map export" in M9, not in v0.1 (m9-12).
- **H10. m5-12:** a "questions to ask of this map" page linked from the plan card.

### Approximation and Rust practice ([sota-approx-rust.md](sota-approx-rust.md))

- **R1. m3-09's betweenness was wrong as written**: batching it with multi-source bitset BFS
  needs about 84 GB on Friendster (arithmetic: ~20 B per node per source × 64 sources).
  Replaced by KADABRA's bidirectional path sampler with SILVAN's fixed schedule and integer
  hit counts, the RNG keyed by sample index (N5), and a top-k mode for rankings, which the
  "top-100 overlap ≥ 90%" check uses. Multi-source BFS stays for sampled closeness and
  harmonic centrality.
- **R2. HyperBall's bit-stability rules** in m3-09: a double buffer, stopping at
  stabilization, a seeded hash, Ertl's estimator (no `ln`).
- **R3. HyperBall over budget:** counters partitioned by blocks (Oasis); max-union makes the
  result independent of the partition count. HyperBall stays the default over ADS.
- **R4. The approximate-results contract** (`docs/api-sketch.md` § 2.8 and a
  `design/DECISIONS.md` row): `.approximate` plus `meta["approximate"]` (method, seed,
  samples, ε/δ or RSD, runs); precision parameters are never derived from the budget or the
  thread count; HyperBall's harmonic and closeness print only an empirical spread.
- **R5. Safe helpers before m2-08, m4-01 and m4-03 copy the lab's `SyncPtr`:**
  `par::for_each_range_mut` with a check that offsets increase, and a deterministic parallel
  sort for single indirect writes. The lab's `fetch_add` member scatter gives a member order
  that depends on the schedule and must not reach results. New issue m2-28, a dependency of
  m2-08.
- **R6. D13's open question:** an A/B behind the parallel facade, rayon against a static
  worker pool (SPAA'24: Rust/Rayon scaled worse than PBBS/OpenCilk on all 14 benchmarks at 24
  cores), added to m1-08 (#22).
- **R7.** `simd.rs`'s `unsafe` could shrink with Rust 1.87's safe intrinsics: a note in the
  codec issue for the next codec change (m1-16), low priority.
