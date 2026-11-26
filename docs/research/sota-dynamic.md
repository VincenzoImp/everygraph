# State of the art: graphs that change (dynamic storage, snapshots, incremental kernels, stable maps)

Second-wave review, brief W4 of `docs/research/sota-coverage.md` § (d) and topic 4 of § (c),
2026-11-25/26 (Opus; papers read from `/home/tux/graphlab/papers/dynamic/`). Nothing was built,
measured or loaded; every number below is a paper's, on that paper's machine, and is not
comparable to ours as-is. Our side read: DESIGN § 2–5, `design/DECISIONS.md`, FINDINGS
(temporal, properties, B1), `docs/spec/eg-adjacency.md`, `docs/spec/graph-semantics.md`,
the container spec on branch `m2-01-container-spec` (PR #170), issues m2-01, m2-07, m2-08,
m2-10, m2-12, m3-01, m3-04, m4-06, m5-01, m5-03, m5-08, m7-04, m7-05, m9-04, and the
first-wave reports (storage, communities, maps, ingest-query, kernels, io-memory).

---

## 0. What `.eg` and the adjacency format must fix or reserve now (PR #170, before v1.0)

**Good news first.** The container of PR #170 is already the right shape for an
append-only, LSM-style graph: sections are only ever added, a commit is "new sections + new
TOC + header flip", unknown kinds are skipped, required feature bits make an old reader
refuse what it would misread, TOC entries can grow, and readers of an earlier commit are
never disturbed (§ 8, § 10, § 11). LSMGraph (SIGMOD 2024), GraphOne (FAST 2019) and
CommonGraph (ASPLOS 2023) all converge on what that protocol gives for free: an immutable
compact base plus immutable delta runs, merged in the background. Appending a segment is one
ordinary commit. **No change to the header, the TOC layout or the commit protocol is
needed.** What is missing is (a) one real defect between two specs that blocks the event
model on which time windows and segments rest, and (b) a handful of reservations and
definitions that cost nothing now and avoid a major version later.

### 0.1 Defect to fix now: the default event model cannot be stored by adjacency v1

`docs/spec/graph-semantics.md` § 2.5: "The adjacency stores **one entry per event** … within a
node's list, entries are sorted by (neighbour in graph order, time, input order) … so the
events of a pair are contiguous". `duplicates="events"` is the **default** (A1). But
`docs/spec/eg-adjacency.md` § 1 codes lists "strictly increasing" (`w_j = x_j − x_{j−1} − 1`),
and the encoder refuses anything else (`crates/everygraph-format/src/adjacency/encode.rs`,
`EncodeError::NotIncreasing`). Any input with a repeated pair — every transaction, message or
citation-with-time dataset, i.e. exactly the temporal ones — cannot be written. Nothing in
the format, m2-08 (#35) or m2-10 says where multiplicities and per-event times live.

Two ways out; I recommend (A).

- **(A) Pairs in the adjacency, events in a layer beside it.** The adjacency keeps its strict
  per-pair lists (the analytic graph every kernel reads, at today's bits per entry); a new
  section gives, per pair entry, the range of its events; event columns (time, per-event
  weight) are indexed by event offset, pair columns (multiplicity-derived aggregates) by
  entry offset. Kernels never touch events; windows, `edges()` and event counts do. On
  Ethereum-like data with many events per pair, kernels scan pairs only (a multiset codec
  would make every kernel scan and dedupe every event). It is also what segments need: an
  append adds events to existing pairs without touching the base's pair lists.
- (B) Allow equal neighbours (`w_j = x_j − x_{j−1}`, a flag). One section fewer, but every
  kernel pays a per-entry dedupe and multiplicity count, and appended events for an old
  pair cannot live in a separate run without re-sorting.

Spec sentences for (A):

> *eg-adjacency § 3 (new paragraph).* "An adjacency stores **pairs**: the list of `v` holds
> each neighbour once. When the graph has more than one event for some pair, the section
> `<name>.events` (kind **E**, below) holds an Elias–Fano sequence of `len = entries + 1`,
> `U = events`: for each entry, in entry order, the number of events of the pairs before it,
> then `events`. The multiplicity of entry `e` is `ev[e + 1] − ev[e]`; per-event columns are
> indexed by event offset, per-pair columns by entry offset. A graph whose every pair has
> one event has no `.events` section, and its event offsets are its entry offsets."

> *graph-semantics § 2.5 (replace the first sentence).* "The adjacency stores one entry per
> pair, in graph order of the neighbour; the events of a pair are stored, in (time, input
> order) order with nulls of time last, in the event columns, at the event offsets the
> adjacency's `.events` section gives (`eg-adjacency.md` § 3)."

Cost: on the four social graphs measured (no repeated pairs) zero; on a graph with
`events/pairs = r` the `.events` sequence is about `2 + log2(r)` bits per pair. Owner: #30
(spec), #35 (m2-08, build), #37 (m2-10, columns). This needs fixing whether or not segments
are ever built.

### 0.2 Reservations in the container (eg-container.md)

| # | Where | Sentence to add | Why |
|---|---|---|---|
| C1 | § 10, required features | "Bit 0 of the required features, `segments`, is reserved: it is set exactly when the graph of the file is the union of more than one adjacency run (§ 13). A file without it is read as today; a 1.0 reader refuses a file with it." | Without the bit, a 1.0 reader would read only the base run and return an *older graph* silently — the one case the required bits exist for. Setting it only when segments exist keeps every unsegmented file readable by 1.0 readers. |
| C2 | § 4, names | "Names beginning with `seg/` are reserved for segments: `seg/` + the segment number as 8 lowercase hexadecimal digits + `/` + the name the section would have in an unsegmented file (`seg/0000002a/adjacency.out.index`). The base run keeps the names of an unsegmented file." | Bytewise name order then groups a segment's sections and orders segments by number; an unsegmented file never changes. |
| C3 | § 5, kinds | Reserve now: **M** = segment manifest (one per file, name `segments`), **E** = event offsets of an adjacency (§ 0.1). Suggest the next two free numbers after the dictionary (5, 6) before #37/#39 take them. | Numbers are cheap; collisions between parallel issues are not. |
| C4 | new § 13 (text only, no implementation in 0.1) | "**Segments.** A segmented file holds a base run and segments `1 … k`, each a complete set of graph sections (adjacency out/in, `.events`, columns, dictionary extension) over the node ids `0 … n_i − 1` known after it. The manifest lists, per segment: number, level (compaction tier), node count after it `n_i`, pairs, events, `t_min`/`t_max` of its events in the time column's unit (absent without a time column), the input fingerprints, and the hashes of its sections. The graph of the file is the union of the events of all runs; a pair's aggregates are computed over its events in every run. Appending writes one segment and commits; merging writes a merged run and commits a TOC without the runs it replaces; `compact` reclaims their bytes." | Makes m9-04 an implementation of a written rule, not a format change. Event-time ranges (not "segment = time slice") because real feeds arrive out of order; as-of T takes whole runs with `t_max ≤ T` and filters runs with `t_min ≤ T < t_max`. |
| C5 | § 6, provenance; and api-sketch § 2.8 | "A result's provenance record holds the parameter `graph`: the **graph identity**, BLAKE3 of the concatenation, in name order, of the hashes of the sections that define the graph (adjacency, `.events`, dictionary, columns; for a segmented file, of the manifest). A reader serves a stored result only for a graph with that identity." | After an append, results stored "on the whole graph" (api-sketch § 2.8) become wrong silently unless identity is recorded now. They stay valid — as results *as of* the earlier segment set — which is what an anchored map update needs (§ 3, maps). The m2-12 cache key "graph content hash" gets its definition. |
| C6 | § 12 | "One commit may add a segment and drop the runs a merge replaced; a reader of an earlier commit keeps reading those runs (§ 11)." | Already true; saying it closes the question for m9-04. |

Nothing else in the container needs reserving: the 16 MiB TOC limit holds ~7,000 segments
at ~20 sections each, far beyond a logarithmic merge policy; opening a segmented file is
the two reads of § 7 plus the manifest.

### 0.3 Reservations and clarifications in the adjacency format (eg-adjacency.md)

| # | Where | Sentence | Why |
|---|---|---|---|
| A1 | § 3, flags | "Flag bit 0, `sparse`, is reserved for segments: the index then holds, after the block codecs, an Elias–Fano sequence of the node ids that have a list (`len = present`, `U = nodes`), and the offset sequences run over present nodes only." | A dense index costs about `2(n + 1)` bits whatever the segment holds: on Friendster (n ≈ 65.6 M) a daily append of 0.01% of the 3.6 B entries (~1 MB of records) would carry ~16 MB of index (arithmetic from § 2–3 of the spec; LSMGraph stores ⟨src, offset⟩ pairs per segment for the same reason). Readers already refuse unknown flags, so reserving the bit is enough. |
| A2 | § 4, blocks | Replace "they make the chunking of scans a property of the file" by: "Blocks are the unit of I/O and of encoding. The order of result-bearing reductions is not taken from them: it is fixed by node ranges that depend on the node count only (`docs/spec/…`, #44), so re-encoding, merging segments or compacting never changes a result." | "Any valid directory may be written" (§ 4) and "chunking of scans a property of the file" contradict each other once two writers (the in-RAM encoder of #30, the streaming one of #35, a segment merge) exist. m3-01 and m3-04 already require framework-owned node-range reduction blocks; the format text should not promise the opposite. |
| A3 | § 1 or § 8 | "A merged run is byte-identical to the run an encoder writes for the same pairs: records, Elias–Fano sequences and the greedy block rule of § 4 are pure functions of the lists." | Then "compacted" and "built in one ingest" are the same bytes for the same history, and `verify` can check a merge. True today; worth stating (Aspen's C-trees need hash-chosen chunk heads to get this property; PaC-trees chose deterministic chunking for the same reason). |

### 0.4 Rules elsewhere that segments touch (graph semantics, dictionary)

| # | Where | Sentence | Why |
|---|---|---|---|
| S1 | graph-semantics § 4.3 | "A pair's sum is the exact sum of its events **in every run**. A run never stores a rounded partial sum of a pair that a later run may extend; `duplicates="sum"` with a floating-point weight makes a graph that cannot be appended (`Unsupported`), integer and decimal sums can." | § 4.3 relies on "a pair's events are contiguous"; across runs they are not, and `round(round(a) + b) ≠ round(a + b)`. Integers/decimals merge exactly. |
| S2 | graph-semantics § 2.7 | "Column statistics are recorded per run and for the whole graph; `sum` and the largest multiplicity of the whole graph are recomputed over all runs, not combined from the runs' values." | Max multiplicity of a pair split across runs is not the max of the runs' maxima; exact `sum` likewise. |
| S3 | m2-07 / api-sketch § 2.2 (graph order) | "In a file built by appends, graph order is the order of the base followed, for each segment in turn, by its new identifiers in the order that segment's input first names them. A fresh ingest of the concatenated inputs gives the same order only when ids are assigned by first appearance." | A sorted-key or rank-bitmap order (dense integers) cannot place a new key between old ones without renumbering every id and every stored result. Saying it now keeps "same input → same output" exact: the input of an appended file is its sequence of batches. |
| S4 | D14 / `design/DECISIONS.md` "What 'repeatable' promises" | "For a file built by appends, results depend on its batches, not on how they are stored: merging or compacting segments never changes a result." | The testable promise segments need; it follows from A2, A3, S1 and the merged-order rule of § 3 below. |
| S5 | m2-07 | "The dictionary of a segmented file is layered: the base dictionary plus, per segment, a dictionary of its new keys with ids from `n_{i−1}`; a lookup probes the layers newest first; a merge may rebuild one dictionary for merged runs." | D8's MPHF is static; rebuilding it per append is O(n) per day. |

---

## 1. Scope and reading log

Scope (W4): dynamic and temporal graph storage against our plan (static compressed canonical
adjacency + immutable time-partitioned segments, m7-05, m9-04); multi-snapshot evaluation;
batch-dynamic kernels and incremental Leiden, and whether they can be deterministic and
budget-independent; map stability after an append. Not redone: Aspen/Terrace as static
containers (storage), Raphtory and Graphsurge as products (ingest-query).

Read levels: **full** = method and evaluation read; **partial** = named sections;
**abstract/results** = abstract, headline results and setup only; **delegated** = read in
full by a helper agent briefed with our questions, its summary checked against the text
for the numbers quoted here.

PDF paths are relative to `/home/tux/graphlab/papers/dynamic/`; text in `txt/`.

| Work | Venue, year | Verified link | Read | PDF |
|---|---|---|---|---|
| Su et al., Revisiting the design of in-memory dynamic graph storage | SIGMOD 2025 (PACMMOD 3(1)) | arxiv.org/abs/2502.10959 | full (§ 4–8) | 2025-su-revisiting-dgs.pdf |
| Yu et al., LSMGraph: multi-level CSR | SIGMOD 2024 (PACMMOD 2(6)) | arxiv.org/abs/2411.06392 | full (§ 1, 4.2, 5) | 2024-yu-lsmgraph.pdf |
| Dhulipala, Blelloch, Shun, Aspen (C-trees) | PLDI 2019 | arxiv.org/abs/1904.08380 | full (§ 3, 7) | 2019-dhulipala-aspen.pdf |
| Dhulipala, Blelloch, Gu, Sun, PaC-trees / CPAM | PLDI 2022 | arxiv.org/abs/2204.06077 | partial (§ 10.5 graphs) | 2022-dhulipala-pac-trees.pdf |
| Fuchs, Margan, Giceva, Sortledton | PVLDB 15(6) 2022 | vldb.org/pvldb/vol15/p1173-fuchs.pdf | partial (abstract, § 7) | 2022-fuchs-sortledton.pdf |
| Kumar, Huang, GraphOne | FAST 2019 | usenix.org/system/files/fast19-kumar.pdf | partial (§ 1–4 design) | 2019-kumar-graphone.pdf |
| Afarin et al., CommonGraph | ASPLOS 2023 | cs.ucr.edu/~nael/pubs/asplos23.pdf | full | 2023-afarin-commongraph.pdf |
| Han et al., Chronos | EuroSys 2014 | pacman.cs.tsinghua.edu.cn/papers_cwg/eurosys2014.pdf | partial (§ 1–3, setup) | 2014-han-chronos.pdf |
| Sahu, Salihoglu, Graphsurge | SIGMOD 2021 | arxiv.org/abs/2004.05297 | abstract/results (+ § 3) | 2021-sahu-graphsurge.pdf |
| Iyer et al., Tegra | NSDI 2021 | usenix.org/system/files/nsdi21-iyer.pdf | abstract/results | 2021-iyer-tegra.pdf |
| Afarin et al., Analysis of stable vertex values | arXiv 2025 | arxiv.org/abs/2502.10579 | abstract | 2025-stable-vertex-values.pdf |
| Steer et al., Raphtory | JOSS 2024 (arXiv 2023) | arxiv.org/abs/2306.16309 | full (short paper) | 2023-steer-raphtory.pdf |
| Sahu, DF/ND/DS Leiden | arXiv 2024 | arxiv.org/abs/2405.11658 | full (delegated) | 2024-sahu-df-leiden.pdf |
| Sahu, Heuristic-based dynamic Leiden | arXiv 2024 | arxiv.org/abs/2410.15451 | full (delegated) | 2024-sahu-heuristic-dynamic-leiden.pdf |
| Lin, Xie, Fang, Maintaining Leiden communities (HIT-Leiden) | arXiv 2026 | arxiv.org/abs/2601.08554 | full, proofs skipped (delegated) | 2026-lin-maintaining-leiden.pdf |
| Bokov et al., LD-Leiden | arXiv 2025 | arxiv.org/abs/2502.18497 | method + results (delegated) | 2025-ld-leiden.pdf |
| Zarayeneh, Kalyanaraman, Delta-screening | IEEE TNSE 2021 | par.nsf.gov/servlets/purl/10223359 | method + results (delegated) | 2021-zarayeneh-delta-screening.pdf |
| Sahu, DF PageRank | arXiv 2024 | arxiv.org/abs/2401.03256 | full (delegated) | 2024-sahu-df-pagerank.pdf |
| Liu, Shi, Yu, Dhulipala, Shun, batch-dynamic k-core | SPAA 2022 | arxiv.org/abs/2106.03824 | intro, overview, experiments (delegated) | 2022-liu-batch-dynamic-kcore.pdf |
| Acar, Anderson, Blelloch, Dhulipala, batch-dynamic connectivity | SPAA 2019 | arxiv.org/abs/1903.08794 | abstract, intro (delegated) | 2019-acar-batch-dynamic-connectivity.pdf |
| Hu, Kobourov, Veeramoni, Embedding, clustering and coloring for dynamic maps | PacificVis 2012 / JGAA 18(1) 2014 | yifanhu.net/PUB/HuKobourovVeeramoni2014.18.1.pdf | full (delegated) | 2014-hu-dynamic-maps-jgaa.pdf |
| Mashima, Kobourov, Hu, Visualizing dynamic data with maps | PacificVis 2011 (TVCG 2012) | www2.cs.arizona.edu/~kobourov/pacvis11.pdf | full, conference version (delegated) | 2011-mashima-dynamic-data-maps.pdf |
| Sondag, Speckmann, Verbeek, Stable treemaps via local moves | TVCG 24(1) 2018 | pure.tue.nl/ws/files/90606317/StableTreemapCorrected.pdf | full (delegated) | 2018-sondag-stable-treemaps-local-moves.pdf |
| Vernier et al., Quantitative comparison of time-dependent treemaps | EuroVis 2020 (CGF 39(3)) | arxiv.org/abs/1906.06014 | method, metrics, results (delegated) | 2020-vernier-time-dependent-treemaps.pdf |
| Hahn et al., Stable layout of Voronoi treemaps | IVAPP 2014 | hpi.de (author PDF) | full (delegated) | 2014-hahn-stable-voronoi-treemaps.pdf |
| Archambault, Purchase, On the application of experimental results in dynamic graph drawing | GViP 2014 | ceur-ws.org/Vol-1244/GViP-paper5.pdf | full (delegated) | 2014-archambault-application-dynamic-gd.pdf |
| Beck et al., State of the art in visualizing dynamic graphs (EuroVis STAR; CGF 2017 survey not reachable) | EuroVis 2014 | (author PDF) | layout, evaluation sections (delegated) | 2014-beck-star-dynamic-graphs.pdf |
| Yoghourdjian et al., Exploring the limits of complexity | arXiv 2018 | arxiv.org/abs/1809.00270 | relevant parts (delegated) | 2018-yoghourdjian-limits-complexity.pdf |
| Zhu et al., LiveGraph | PVLDB 13(7) 2020 | arxiv.org/abs/1910.05773 | abstract | 2020-zhu-livegraph.pdf |
| De Leo, Boncz, Teseo | PVLDB 14(6) 2021 | vldb.org/pvldb/vol14/p1053-leo.pdf | abstract | 2021-deleo-teseo.pdf |
| Pandey et al., Terrace | SIGMOD 2021 | (course mirror, users.cs.utah.edu) | abstract/results | 2021-pandey-terrace.pdf |
| Qi et al., LSGraph | EuroSys 2024 | theqihao.github.io/files/LSGraph.pdf | abstract | 2024-qi-lsgraph.pdf |
| Xie et al., RadixGraph | arXiv 2026 | arxiv.org/abs/2601.01444 | abstract | 2026-radixgraph.pdf |
| Hao et al., RapidStore | arXiv 2025 | arxiv.org/abs/2507.00839 | abstract | 2025-rapidstore.pdf |
| Macko et al., LLAMA | ICDE 2015 | doi.org/10.1109/ICDE.2015.7113298 | not accessible (author and DASH copies 404/HTML); described via Su et al. and LSMGraph | — |
| Shi, Wang, Xu, Spruce | SIGMOD 2024 (PACMMOD 2(1)) | doi.org/10.1145/3639282 | abstract only (search result; no official open copy) | — |
| Archambault, Purchase, Mental map preservation helps user orientation | GD 2012 | — | not accessible (paywall, mirror 403); results via GViP'14 and Beck | — |
| Greene, Doyle, Cunningham, Tracking community evolution; Brandes, Mader GD'11; Frishman, Tal TVCG'08 | — | — | not accessible / not searched (search budget) | — |

No OpenAlex request was made; ~25 web searches in total (both helpers included).

---

## 2. The state of the art, as a map

### 2.1 Dynamic graph storage

| Family | Representatives | Idea | Best at | Price (paper's numbers) |
|---|---|---|---|---|
| Purely functional compressed trees, coarse snapshots | Aspen (PLDI'19), CPAM/PaC-trees (PLDI'22) | vertex tree + per-vertex trees of difference-coded chunks; a snapshot is a root pointer; single writer, batch updates | large parallel batches (Aspen 105–442 M updates/s for batches of 2 B, 72-core 4× E7-8867 v4, 144 threads); snapshot isolation for free | memory: Aspen 9.42 GB vs Ligra+ 5.1 GB on Twitter (2.4 B entries), 1.2–1.7× (mean 1.51×) slower than Ligra+ on global algorithms; Friendster 19.1 GiB (Aspen) and 14.6 GiB (PaC-tree) for 3.61 B entries = **45 and 35 bits/entry**, against our 23.2 (D4) |
| Fine-grained MVCC, in place | LiveGraph (VLDB'20), Teseo (VLDB'21), Sortledton (VLDB'22), Spruce (SIGMOD'24), RadixGraph (2026), RapidStore (2025) | per-edge versions + locks; sorted blocks (~256) under a skip list / fat tree / PMA | single-edge transactional inserts: Sortledton 4.8 M checked inserts/s, Teseo 3.1 M (graph500-24, 56 threads, dual Xeon) | Su et al.: version checks per neighbour, 4.1–8.9× CSR memory for the best of them; Sortledton's own number 2.1× CSR (18 vs 8.3 GB, graph500-24) |
| PMA / hierarchical containers | Terrace (SIGMOD'21), BYO (VLDB'24) | degree-adaptive: inline / PMA / B-tree per vertex | batch inserts ≤ 1 M faster than Aspen; queries 1.7–2.6× faster than Aspen, 0.5–1.3× of Ligra (24 cores, Xeon 8275CL) | in-memory only, uncompressed |
| Log + compact base (LSM, delta runs) | LLAMA (ICDE'15), GraphOne (FAST'19), LSMGraph (SIGMOD'24), CommonGraph's representation (ASPLOS'23) | immutable CSR(-like) runs + an in-memory buffer; runs merged by level (LSMGraph: one CSR per level, T = 10, ≤ 5 levels, MemGraph 64 MB) | disk-resident growth with sequential writes; LSMGraph: 2.85× LLAMA's insert throughput and 3.1× its analytics, 16 vCPU, 16 GB cgroup, 500 MB/s SSD, IT/UK/Twitter/Friendster | reads merge runs (LLAMA: "a large number of CSR fragments", LSMGraph adds a multi-level index); no paper compares against a static compressed CSR on the same data |

**The systematic study.** Su et al. (SIGMOD 2025) re-implement LiveGraph, Teseo, Sortledton,
Aspen and LLAMA in one framework (EPYC 7543, 32 cores, 128 GB; kernels single-threaded):
CSR is **2.4–11.0× faster on queries and 3.3–10.8× smaller** than the dynamic stores; on
lj, PageRank takes 4.66 s on CSR against 13.06–13.72 s for the best configurations of
Aspen, Teseo and Sortledton (Table 5); segmented containers cost more cache misses,
branches and instructions (Table 6). Coarse-grained snapshots (Aspen) win once batches
exceed ~2^8 updates; fine-grained ones win for single updates. They omit Friendster
"since existing DGS methods frequently run out of memory". Sortledton's own paper reports
1.22× of CSR on average — the gap depends on who measures and on single vs many threads.

**For a read-mostly analytics engine that grows by batches**, every line of evidence points
the same way: keep the read path a static compact array format; take updates as immutable
batches; merge in the background with a logarithmic policy (Bentley–Saxe, which LSMGraph
cites as its basis). Per-edge versioning and in-place containers buy what we do not sell
(transactions, single-edge latency) with 2–10× memory.

### 2.2 Many snapshots

| System | Idea | Numbers (paper) |
|---|---|---|
| Chronos (EuroSys'14) | time-locality layout: an edge carries the set of snapshots containing it; per-vertex state of all snapshots stored together; one pass per iteration updates every snapshot (LABS) | > 10× over per-snapshot runs single-threaded; > 2× over embarrassingly parallel per-snapshot runs on 16 cores (2× Xeon E5-2665); Wikipedia and other temporal graphs |
| CommonGraph (ASPLOS'23) | compute on the intersection of the snapshots (a CSR never mutated), then add each snapshot's missing edges as separate CSR batches; deletions become additions; share additions via a Steiner tree on a "triangular grid" | 1.38–8.17× over KickStarter, 50 snapshots × 75 K changes (half deletions), monotone kernels only (BFS, SSSP, SSWP, SSNP, Viterbi); 56-core Xeon E5-2680, 520 GB |
| Graphsurge (SIGMOD'21) | view collections run in Differential Dataflow; order views to minimise differences (NP-hard; approximation); split: run some views from scratch | ordering up to 10.1×; splitting picks differential for BFS and from-scratch for PageRank on the same collection |
| Tegra (NSDI'21) | versioned persistent state store + incremental computation over arbitrary windows (Timelapse) | up to 30× on ad-hoc window workloads (abstract) |
| Stable vertex values (2025) | many vertex results never change across snapshots; bound them by intersection/union analysis | 53.2–99.8% of vertices unchanged over 25–100 snapshots for vertex-specific queries (abstract) |
| Raphtory (JOSS 2024) | in-memory temporal graph, windows, rolling/expanding views, layers, deletion semantics | no performance claims in the paper |

**What transfers.** For **as-of** queries on an append-only graph, CommonGraph's problem
disappears: every earlier snapshot is a subset of every later one, so the "common graph" is
the earliest snapshot and *every* transition is an addition. Monotone kernels (components by
union-find, reachability, BFS levels, k-core lower bounds… see § 3.3) can then produce
results at 100 time points in **one pass over the events in time order**. For
non-monotone iterative kernels (PageRank), Chronos's batched layout is the way to run k
snapshots for the price of one scan per iteration, at k × the vertex state — a cost our
planner can budget exactly.

### 2.3 Batch-dynamic kernels and incremental communities

| Work | Keeps between batches | Numbers (paper's machine) | Deterministic? | Equals the from-scratch result? |
|---|---|---|---|---|
| DF / ND / DS Leiden (Sahu 2024) | memberships, degrees, community totals; only pass-1 local moving is incremental, later passes full | mean 1.98× (DF) over static, 3.72× at batches of 10^-7 \|E\|, 1.33× on 5 temporal graphs; Q within 0.002; 64 threads, EPYC 7742 | no: asynchronous moves, atomics, racy representative choice; the author says asynchrony "increas[es] variability" | not tested (batches applied independently to the base) |
| Heuristic dynamic Leiden (Sahu 2024) | + per-community cumulative ΔΣ, refine only above τ_re = 0.6; old-id tracking | 6.1× (DF) mean, Q within 0.002, same machine | no (adds random new ids, CAS) | **measured: no.** Delete a batch then reinsert it: memberships match static Leiden on the identical graph 84% (batch 10^-7 \|E\|) down to 44% (0.1 \|E\|) with tracking, "near zero" without |
| HIT-Leiden (Lin, Xie, Fang 2026) | the whole hierarchy, per-level mappings, a dynamic connectivity index per level | "up to five orders of magnitude" over DF-Leiden; Q within 0.01; over 999 batches modularity *more stable* than reruns; production 7.7× (58.3 M nodes, 0.5 M updates/day) and 56.1× (Graph-RAG); Xeon 8336C | **sequential** implementation; parallel is future work | no claim |
| LD-Leiden (Bokov et al. 2025) | hierarchy with exact aggregates (subtract-then-add) | 1 thread: 48.77× over warm-started leidenalg at Q ratio 0.996; 6.94× over DF-Leiden; 5.22× from 1 to 64 threads (2-socket, 96 cores) | nearly: moves from a frozen snapshot, sorted by gain, serial give/receive filter; still **Q spread 0.38% mean, 1.49% worst across 1–64 threads** (DF-Leiden 5.11% / 11.87%) | "path-dependent … not guaranteed to reproduce … a full Leiden rerun"; recommends periodic full refresh |
| Delta-screening (Zarayeneh, Kalyanaraman 2021) | Louvain/SLM seeded on screened vertices; sequential | up to 38× over static, Threadripper 1920X | yes (sequential, sorted batch) | order within one batch changes Q: 0.658227 (deletions first) vs 0.652412 (additions first) |
| DF PageRank (Sahu 2024) | previous ranks; frontier grows when a rank moves by > τ/10^5 | 7.8× over static, 2.9× over naive warm start, 64 threads, EPYC 7742, batches 10^-7–0.1 \|E\|; on temporal graphs needs a looser frontier tolerance | async in the paper; the paper itself notes synchronous rounds are deterministic | no: error vs a τ = 10^-100 reference is reported, not identity |
| Batch-dynamic k-core, PLDS (Liu et al. SPAA'22) | O(log² n) levels; synchronous level-by-level moves | up to 114.52× over parallel exact (friendster, twitter; 30 cores, 60 threads); (2+ε)-approximate, mean error 1.26–2.13 | deterministic variants given | no: approximate and history-dependent |
| Batch-dynamic connectivity (Acar et al. SPAA'19) | HDT levels on batch Euler-tour trees | theory only | randomized | partition yes (canonical); labels if min-id |

**Two facts decide our design.** (1) *Nondeterminism in these codes is an implementation
choice, not a property of the methods*: frontiers are seeded from batch endpoints, moves can
be taken in synchronous rounds with a total-order tie-break (LD-Leiden is one step away), and
thresholds compare values that are bit-identical if reductions are ordered. So a
deterministic incremental Leiden for any thread count and budget is feasible, at roughly the
cost we already pay for D14 (2.4–3.0×, D14), on a much smaller frontier. (2) *Path
dependence is real and measured*: no incremental community method returns the from-scratch
partition of the final graph, and even sequential ones depend on the order of updates inside
a batch. The first wave's dismissal ("not deterministic; out of scope", sota-communities
§ 6) is right about the codes and wrong about the method; the true obstacle is that D14's
"same input" must be redefined as *same base and same sequence of batches* for any
incrementally maintained result.

**Exactly incremental kernels for appends** (insert-only, the m9-04 model): degrees, event
counts, per-pair aggregates with exact types, connected components by union-find with
min-id labels (the partition is a function of the graph, so incremental = from scratch),
reachability/BFS levels by relaxation, and exact core numbers if an exact insertion
algorithm is used (coreness rises by at most 1 per inserted edge — Sariyüce et al., not read
here). Not exactly incremental: PageRank (a warm start converges to a different
floating-point point at any tolerance), Leiden, layout.

### 2.4 Dynamic drawing and the mental map

| Work | Method | Stability measure and numbers |
|---|---|---|
| Hu, Kobourov, Veeramoni (PacificVis'12 / JGAA'14) | warm-started clustering (first Louvain level merges only pairs that were together before); colours by max-weight bipartite matching of cluster overlaps (Hungarian); layout from previous positions + Procrustes, scale blended (s = 0.5) because it compounds; new nodes start at their closest surviving node | Rand distance 0.0631 → 0.0252 (DNA, 39 nodes, 100 steps); nodes keeping colour 16.4 → 34.27 of 39 (DNA) and 4.55 → 78.27 of 100 (TV); mean travel 21.41 → 13.19 → 8.43 px (independent, initialised, Procrustes) |
| Mashima, Kobourov, Hu (PacificVis'11 / TVCG'12) | one fixed "canonical map" (18,000 last.fm artists), refreshed "once a month"; frames show the top 250 on it | "Too much of a good thing (mental map preservation) can be bad": change became invisible, so they added a heat map and staged appear/disappear frames |
| Sondag, Speckmann, Verbeek (TVCG'18) | keep the old treemap, repair by local moves; accept a move only if quality improves by c = 4 (hysteresis); level by level | relative-position change (pairwise, 8 sectors); beats all but slice-and-dice on stability; after insertions/deletions layout "virtually the same"; 62 ms/step on 86 rectangles |
| Vernier et al. (EuroVis'20) | 14 treemap algorithms on 2,405 datasets | corner travel normalised by the map diagonal; data-aware σ against a "keep the old structure, new areas" baseline; state-aware methods are stable but their quality decays; "no algorithm performs best on both" |
| Hahn et al. (IVAPP'14) | Voronoi treemap sites seeded from a hash of each node's path → stable across revisions | visual only (Chromium revisions) |
| Archambault & Purchase (GD'12, PacificVis'12, TVCG'11; review GViP'14) | user studies | stability helps **map-like tasks** (locate nodes, follow long paths) when ≥ 5 un-highlighted targets matter; no effect on memorability; small multiples faster than animation; all on ≤ 60-node graphs (Yoghourdjian: 80% of 152 studies use ≤ 100 nodes) |

**State of the art in one line:** anchor on the previous drawing, match clusters by overlap
for identity and colour, accept a moved layout only with hysteresis, relayout on a schedule
or when quality decays, and show the change explicitly. Nobody in what we read does it
above 18,000 nodes (Mashima; the others ≤ 372 elements), nor for hierarchical disc packing.

---

## 3. Decision by decision

| Decision / issue | Best known work | Us | Evidence |
|---|---|---|---|
| **D4** compressed static adjacency as the canonical store | Aspen/CPAM keep graphs compressed *and* updatable; LSMGraph keeps CSR runs on disk | **Ahead on space, right on structure.** Friendster: ours 23.2 bits/entry (D4), PaC-trees 34.7 and Aspen 45.4 (3.61 B entries; arithmetic on PaC-trees Table 4, GiB). Every dynamic store reads slower than a static array (Su et al.: CSR 2.4–11.0× faster, single-threaded) | Keep D4 as the format of *every* run, base and segment. |
| D4 / eg-adjacency **lists strictly increasing** vs graph-semantics § 2.5 "one entry per event" | multigraph = pair CSR + multiplicity/edge-id arrays (Kùzu-style edge columns; LSMGraph stores per-edge records) | **Wrong today** (§ 0.1): the default `duplicates="events"` cannot be encoded | `encode.rs` refuses equal neighbours; no section holds multiplicities or per-event offsets. |
| eg-adjacency § 4 "blocks make the chunking of scans a property of the file" | — | **Inconsistent** with "any valid directory may be written" and with m3-01/m3-04 (framework-owned node-range reduction blocks) | Becomes a real hazard once a merge or a second encoder writes other block directories (§ 0.3 A2). |
| **D7** `.eg` container | LSMGraph: immutable CSR runs + levelled merge in separate files; GraphOne: durable edge log + archived adjacency; LLAMA: snapshot fragments | **Ahead for our needs**: one file, append-only commits, A/B headers, readers of older commits undisturbed — an LSM in one file | PR #170 § 8, § 11. Needs the reservations of § 0.2 (required bit, `seg/` names, manifest kind, graph identity). |
| **D8** id dictionary (static MPHF, rank bitmap) | LSMGraph/LiveGraph append vertices to a vertex file and recycle ids; no static-MPHF layering found in the graph papers | **Behind for appends**: an MPHF and a sorted rank cannot take new keys | Layered dictionary + graph order = base order then new keys by first appearance (§ 0.4 S3, S5). |
| **D9** hot per-entry columns | per-edge records with timestamps (LSMGraph `⟨dst, ts, offset, marker⟩`); Chronos per-edge snapshot sets | **Level** for pairs; **unspecified** for events | With § 0.1 (A): per-pair columns at entry offsets, per-event columns (time, event weight) at event offsets. |
| **D10 / m7-04** views; window as a view | Tegra/Graphsurge: windows as first-class, shared state across many windows | **Level** for one window; **behind** for many | A window over segments takes whole runs and filters only boundary runs; per-block min/max time in the time column lets a filtered scan skip blocks (also sota-ingest-query). |
| **D14** deterministic for any threads, budget, regime | No dynamic store or incremental algorithm read discusses determinism except DF PageRank (one sentence) and LD-Leiden (measures 0.38% Q spread over threads) | **Ahead in intent, undefined for appends.** Two gaps: (a) results must not depend on how runs are merged — per-node float reductions over neighbours must visit the canonical merged order (or use order-independent sums, W1); (b) incrementally maintained results are path-dependent (measured: 44–84% match after delete-reinsert; Q 0.658 vs 0.652 by order within one batch) | § 0.4 S4; incremental results keyed by the batch sequence. |
| **D15** block-permuted visiting order | DF/LD-Leiden: frontier from batch endpoints | **Compatible**: a frontier visited in the same block-permuted order keeps one code path in RAM and out of core | — |
| **D16 / m4-06** hierarchy artifact with exact aggregates | HIT-Leiden and LD-Leiden keep the full hierarchy and repair per-level aggregates by subtract-then-add | **Level, and the right state**: m4-06 already stores per-level quotient graphs with integer weights and exact volumes — exactly what an exact incremental repair needs | Integer weights make the repair exact; f32 quotient weights (m4-06's weighted case) would not. |
| **D17 / m5-01** deterministic layout | Hu et al.: warm start + Procrustes; Sondag: local moves with hysteresis; Hahn: hash-seeded sites; Vernier: stability and quality trade off, state-aware layouts decay | **Behind** (no update story); **D17's determinism must be restated** for updates: an anchored map is a pure function of (new graph, previous map file), not of the graph alone | Mashima's canonical map refreshed monthly is the simplest deterministic policy. |
| **D18 / m5-03** tiles | — | m5-03 lists "incremental rebuild of subtrees" as a stretch | Stable region identity (Hu's matching) keeps tile keys of unchanged regions; only touched subtrees rebuild. |
| **m7-05** windows now, segments designed | Raphtory: windows, rolling/expanding views, layers; Chronos: batched snapshots | **Level** for v0.1 scope (one window, time column) | The 5× target on a 10% window is a materialized view (D10 lab: 0.09–0.17× per PageRank iteration at 1–10%). The design note must cover § 0. |
| **m9-04** segments and appends | LSMGraph (levelled runs, multi-level index, T = 10), CommonGraph (base + immutable addition batches), GraphOne (log + archive) | **Right direction, missing pieces**: event layer, sparse segment index, merge policy, merged-order reads, layered dictionary, exact cross-run sums, as-of sweeps, history-keyed incremental results | Acceptance "append 1% in time proportional to new data": true for the topology and columns (sort Δ events, probe the dictionary, write one run); **not** for hierarchy, layout and tiles unless they are incremental (§ 4 C6–C7). |
| **m2-12** artifact cache | — | "graph content hash" undefined | § 0.2 C5: graph identity from section hashes; incremental results additionally keyed by the ordered batch list. |

---

## 4. Challenges, ranked

**C1. Fix the event model in the format now (§ 0.1, option A).**
*Change:* adjacency over pairs + `<name>.events` Elias–Fano of per-entry event offsets;
per-event columns at event offsets. *Gain:* the default ingest becomes encodable for every
input with repeated pairs (today: refused); kernels scan pairs, not events. *Cost:* one
section kind, a few hundred lines in #30/#35/#37, two spec paragraphs; no risk to
determinism, budget or wasm32. *Experiment:* on a temporal dataset with repeats
(sx-stackoverflow or wiki-talk-temporal from SNAP, or the m8-06 blockchain sample), count
events/pairs and the `.events` bits per pair; interleaved A/B of a pull PageRank iteration
on (A) pairs vs (B) a multiset codec with on-the-fly dedupe, 16 threads, idle machine.
*Issues:* follow-ups to #30 (m2-03), #35 (m2-08), #37 (m2-10); graph-semantics § 2.5.

**C2. Put the reservations of § 0.2–0.3 into PR #170 and `eg-adjacency.md` before v1.0.**
*Change:* required bit 0 `segments`; `seg/` names; manifest and event-offset kinds;
graph identity in provenance; `sparse` flag bit; § 4 block wording. *Gain:* m9-04 needs no
major version; stored results never served for a grown graph. *Cost:* text only. *Risk:*
none. *Experiment:* none needed; the Python reader of #28 should list a hand-made file with
bit 0 set and refuse it. *Issue:* #28 (PR #170), #30.

**C3. State and test D14 for appended files: "merging or compacting never changes a result".**
*Change:* per-node reductions over neighbours visit the merged canonical order (k-way merge
per list; most lists live in one run) or use order-independent sums (W1); reduction
blocks are node ranges fixed by node count (§ 0.3 A2); pair sums over all runs (§ 0.4 S1).
*Gain:* a promise no dynamic store makes; a test that is cheap to run in CI. *Cost:* merge
cost on reads (LSMGraph keeps it small with ≤ 5 levels and a multi-level index). *Risk:*
none if built in; costly to retrofit. *Experiment:* oracle graphs split into 1–5 runs at
random cut points, hash of PageRank/WCC/Leiden on split vs merged, 1 and 32 threads.
*Issue:* new, "Segments: compaction-invariant results" (M9, before m9-04 code).

**C4. Specify m9-04's storage as a levelled log of compressed runs.**
*Change:* runs in the D4 format with the sparse index; Bentley–Saxe/levelled merge (each
level ≥ T× the one above, T ≈ 10 as LSMGraph; a merge is a pure function of its inputs,
§ 0.3 A3); manifest with event-time ranges; as-of = whole runs with `t_max ≤ T` + filtered
boundary runs. *Gain:* append cost ∝ Δ (amortized O(log_T(m/Δ)) rewrites per event), reads
within a small factor of one run. *Cost:* merge code in the edge-map sources (#44 must allow
a multi-run source from the start — the one design point that is hard to add later).
*Experiment:* lab, lj and orkut with a fixed random time stamp per edge, base 90% + 10
runs of 1%: PageRank iteration and BFS on k = 1, 2, 4, 11 runs vs compacted, 16 threads,
idle. *Issues:* m9-04; #44 (m3-01): "AdjacencySource may be a merge of runs".

**C5. As-of sweeps instead of 100 window runs.**
*Change:* for monotone kernels (WCC by union-find, reachability, BFS levels, degrees, event
counts) one pass over events in time order yields results at every requested time point
(CommonGraph's additions-only case, which appends give for free); for PageRank at k time
points, Chronos-style batched state (k values per node, one scan per iteration), budgeted
as k × n × 8 bytes by the planner. *Gain:* Chronos > 10× single-threaded, > 2× on 16 cores
over per-snapshot runs; CommonGraph 1.38–8.17× (both on their machines). This is what m9-04's
acceptance ("100 time points without rescanning") needs. *Cost:* a time-sorted event order
(a permutation or a time-sorted copy of the event layer). *Risk:* none for determinism
(union-find with min-id labels is canonical). *Experiment:* WCC component count at 100
time points on a SNAP temporal graph: one sweep vs 100 `window(None, t)` runs.
*Issue:* m9-04 or new "As-of sweeps for monotone kernels".

**C6. Deterministic incremental Leiden over the stored hierarchy (research, after #60).**
*Change:* frontier = sorted ids of endpoints of changed pairs (no tolerance in marking);
synchronous rounds: frozen totals → best moves → sort by (gain, id) → LD-Leiden's
give/receive filter → commit; refine only flagged communities; repair per-level aggregates
exactly (integer weights, m4-06); deterministic fallback to a full rebuild (fixed fraction
of changed weight or of frontier, and always at compaction). *Gain:* DF-Leiden 1.98× mean,
heuristic variant 6.1× (64 threads, EPYC 7742); HIT-Leiden orders of magnitude for small
batches (sequential). *Cost:* high; and results become **history-keyed** (path dependence
measured above), which D14 and the cache must say. *Experiment:* our deterministic Leiden
with a warm-start/frontier option, lj and orkut, 90% base + ten 1% batches: hashes at 1/16/32
threads, Q and ARI vs static on the final graph, A-then-B vs A∪B, time vs full run.
*Issue:* new, "Incremental community update: deterministic, history-keyed (research)".

**C7. Anchored map update with stable identity.**
*Change:* match new to old communities top-down by maximum-weight overlap (fixed
tie-break), inherit id, colour and tile key; matched regions keep their centre (radius
rescaled); siblings re-solved from old centres, moved only with hysteresis (Sondag's c);
new nodes inside their community's disc next to their strongest neighbour (Hu); new regions
placed by a hash of their id (Hahn); full relayout of a level when its quality falls a fixed
margin below a fresh layout. Report per update: centre displacement / parent diameter,
fraction of nodes keeping top-level region and colour, ARI, and the D17 edge-length ratio.
Highlight what changed (Mashima). *Gain:* the only user evidence says stability helps
locate-and-orient tasks, our core use; Hu's matching kept 78 of 100 node colours vs 4.55.
*Cost:* moderate; thresholds must come from our own sweep (none published). *Risk:* D17
determinism becomes "function of (graph, previous map)". *Experiment:* two maps of one
temporal graph at T and T + 1% events: anchored vs fresh, the metrics above.
*Issue:* new, "Map update after an append: anchored layout and stable region identity".

**C8. Deletions as retraction runs — reserve only.** CommonGraph: deletions cost ~3× additions
in incremental kernels; our event data rarely retracts. Reserve a run flag "retractions"
under required bit 0; do not build. *Issue:* note in m9-04.

---

## 5. What we have that the literature does not — checked

| Claim | Verdict |
|---|---|
| Static compressed base + immutable delta runs | **Not ours**: LLAMA (2015), GraphOne (2019), CommonGraph's representation (2023), LSMGraph (2024). Ours: runs in a 23-bit/entry codec, inside one crash-safe file. |
| One file, append-only commits, readers of old commits undisturbed | Not found for graphs (also sota-ingest-query); the LSM papers use many files. Modest novelty. |
| Results identical whatever the merge/compaction state of the runs, any thread count and budget | **Not found** in any dynamic-storage or incremental paper read. Closest prior art: Differential Dataflow's semantics (output at time t = the computation on the input at t), which Graphsurge inherits — not read here. |
| Deterministic incremental Leiden for any thread count | Not found; LD-Leiden is closest and still varies 0.38% in Q across threads. Feasible (§ 2.3), unbuilt. |
| Stable hierarchical map updates at 10^6+ nodes | Not found: the largest dynamic map read is 18,000 nodes with 250 shown (Mashima). The novelty search was not complete (Bubble Treemaps, TwitterScope, circle-packing work not checked for dynamics). |
| Pair adjacency + event layer (§ 0.1) | Not novel in concept (multigraph CSR with multiplicity/edge-id arrays). Do not claim. |

---

## 6. What to ignore, and why

- **LiveGraph, Teseo, Sortledton, Spruce, RadixGraph, RapidStore:** fine-grained MVCC for single-edge transactions; 2–10× CSR memory (Su et al.); not our product (DESIGN § 2).
- **Terrace, LSGraph, PMA containers:** in-RAM, uncompressed, built for streams of small batches.
- **Aspen/CPAM as our store:** 1.5–2× our bits on Friendster and C++; their one idea we need, cheap snapshots, our TOC commits already give.
- **HDT-style fully dynamic connectivity (Acar et al.):** for deletions; appends need only union-find.
- **PLDS batch-dynamic k-core:** (2+ε)-approximate and history-dependent; exact recompute or exact insertion updates instead.
- **DF PageRank as a default:** a warm start is never bit-identical to a cold run; at most a labelled opt-in like `fast`.
- **Tegra, Graphsurge as runtimes:** distributed dataflow stacks; take only view ordering and the differential/from-scratch split.
- **Animation vs small multiples:** a viewer question for later; small multiples were faster in the only study (≤ 60 nodes).
