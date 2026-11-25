# State of the art: coverage audit of the first wave

Audit of 2026-11-25 (Opus, web and bibliographic APIs). Question: which works and topics
relevant to Everygraph fall outside the six first-wave scopes (storage, io-memory, kernels,
communities, maps, ingest-query), or inside one but are likely to be missed, so that a
second wave can be launched. This is not an area review: no paper below was read in full
for this audit. Each entry was checked to exist (DOI, publisher page or arXiv) and is
described from its abstract or a summary of it; the second wave must read them. Nothing was
built, measured or loaded.

## (a) Method

**APIs and what worked.**

| Source | Use | Result |
|---|---|---|
| DBLP search API | venue tables of contents | **blocked**: every request (curl, custom user agent, WebFetch) got an anti-bot challenge page. Not used. |
| Semantic Scholar bulk search (`/paper/search/bulk`, `query=graph*`, `venue=…`, `year=2022-2026`, sorted by citations, ≤ 1000 per venue) | venue sweep | worked, slowly (shared rate limit, many 429s). |
| Semantic Scholar citations endpoint | snowballing | **failed**: rate limit; only Ligra completed (1,067 citing papers, 405 since 2022) before I stopped it. Replaced by OpenAlex. |
| OpenAlex `works?filter=cites:W…,publication_year:2022-2026&sort=cited_by_count:desc` | snowballing, 19 anchors | worked (≈ 23 requests; the free daily budget is $0.10 shared by every agent on this IP at $0.001 per request, so I kept to ≈ 50 requests in total). |
| OpenAlex `title_and_abstract.search` | 24 topic queries for the candidate missing topics | worked; noisy for generic terms (e.g. "benchmarking", "sampling"), useful for specific ones. |
| Web search | verifying venue, DOI and claims of ≈ 35 works | worked. |

**Venues swept** (S2 venue names, 2022–2026, titles containing "graph", plus "network" for
the visualization venues): SIGMOD (SIGMOD Conference + Proc. ACM Manag. Data, 377 hits),
PVLDB 521, ICDE 658, PPoPP 55, SPAA 68, SC 20 (**2025 only**: earlier SC proceedings are not
indexed under that venue name), IPDPS 128, EuroSys 30, OSDI 17, SOSP 0, ATC 22, FAST 6, ASPLOS
96, ALENEX 26, ESA 524 (contaminated by an "Embedded Systems and Applications" homonym),
SEA 54, KDD 1000 (capped), WWW 1000 (capped), TVCG 659, Computer Graphics Forum (EuroVis)
439, Graph Drawing 314, DCC 22, SIGIR 424. Not swept: arXiv listings (the arXiv API returned
429); arXiv papers enter only through citations and searches.

**Filtering.** 5,800 raw hits → 1,869 distinct titles after keeping "graph/network" and
dropping GNN, ML, GPU/FPGA, distributed, vector-search and similar titles. A keyword
classifier assigned each title to one of the six areas or "none" and tagged five candidate
missing topics (dynamic, testing, benchmarking, approximation, reproducibility); I then read
the titles by hand, venue by venue, for anything relevant. Counts: none 1,064 (mostly
application mining in KDD/WWW/SIGIR), ingest-query 281, communities 197, maps 165, storage
76, kernels 66, io-memory 20. The S2 venue sweep is **incomplete for small venues** (FAST 6,
SOSP 0, ALENEX 26 graph titles in five years is below the real number), so absence from the
table below is not evidence of absence.

**Snowball anchors** (OpenAlex, citing works 2022–2026, top 25 by citations): Ligra, GBBS
(TOPC 2021), GraphChi, Leiden, GVE-Leiden, WebGraph, Ligra+, Stream VByte, PtrHash, Gorder,
ForceAtlas2, GMap, vmcache, Mosaic, Blaze, Aspen, Hoefler–Belli SC'15, Mytkowicz ASPLOS'09,
Demmel–Nguyen "Parallel reproducible summation". Crotty et al. (CIDR'22) and LUMOS (ATC'19)
could not be resolved (no DOI, title search missed). Leiden and ForceAtlas2 citations are
dominated by single-cell biology and gave one useful lead (GraphRAG).

**Coverage check against the first wave** (§ 4 of the brief): at the time of writing four
reports existed (`sota-storage.md`, `sota-maps.md`, `sota-communities.md`,
`sota-ingest-query.md`, all drafts in progress); io-memory and kernels had not been written.
"Likely missed" below means: not named in the existing report of that area, by string search
on 2026-11-25 ≈ 23:00; it is a hint, not a verdict, since the drafts were still growing.

Raw API dumps were kept in scratch only and deleted at the end (machine rule).

## (b) Important works outside the six scopes, or inside but likely missed

Area: the first-wave area it belongs to, or "none". Status: "outside" = no first-wave scope
covers it; "missed?" = inside a scope, absent from that report's draft; "not checkable" =
the area's report did not exist yet.

| Work | Venue, year | Verified link | Area | Status | Why it matters for us |
|---|---|---|---|---|---|
| **Numerical determinism** | | | | | |
| Demmel, Nguyen: Fast reproducible floating-point summation | ARITH 2013 | doi.org/10.1109/arith.2013.9 | none | outside | Bit-identical sums for any order/partition at ~1 pass; basis of ReproBLAS. |
| Demmel, Nguyen: Parallel reproducible summation | IEEE TC 2015 | doi.org/10.1109/tc.2014.2345391 | none | outside | Same, parallel; the cost model we need for D14. |
| Ahrens, Demmel, Nguyen: Algorithms for efficient reproducible FP summation | ACM TOMS 2020 | doi.org/10.1145/3389360 | none | outside | Current binned-sum algorithms and their cost. |
| Müller, Arteaga, Hoefler, Alonso: Reproducible FP aggregation in RDBMSs | ICDE 2018 | arxiv.org/abs/1802.09883 | none | outside | Reproducible GROUP BY SUM; abstract reports 1.9–2.4× on the aggregation, 2.7% end to end in MonetDB. Closest analogue to per-community weight sums. |
| Xie et al.: FPRev, revealing FP accumulation orders | USENIX ATC 2025 | usenix.org/conference/atc25/presentation/xie | none | outside | Tool to test what order a library/hardware sums in; directly usable in the determinism CI (#54). |
| Sibidanov, Zimmermann, Glondu: The CORE-MATH project | ARITH 2022 | doi.org/10.1109/ARITH54963.2022.00014 | none | outside | Correctly rounded libm (MIT): the only way to use exp/log/sin in result-bearing code and stay identical across platforms (m3-11 currently bans platform maths). |
| Salmon et al.: Parallel random numbers: as easy as 1, 2, 3 (Philox) | SC 2011 | doi.org/10.1145/2063384.2063405 | none | outside | Counter-based RNG: random draws keyed by (seed, node, round), independent of thread count. Maps names "counter-based" once; no area owns RNG policy (ADR 0008 found a thread-seeded `fastrand` in a dependency). |
| WebAssembly 3.0: deterministic profile; relaxed SIMD | W3C, 2025 | webassembly.org/news/2025-09-17-wasm-3.0/ ; github.com/WebAssembly/relaxed-simd | none | outside | Relaxed SIMD (FMA, min/max, …) is explicitly nondeterministic; 3.0 defines a deterministic profile. A rule for the wasm build. |
| Boldo, Jeannerod, Melquiond, Muller: Floating-point arithmetic | Acta Numerica 2023 | doi.org/10.1017/s0962492922000101 | none | outside | Reference survey (FMA, error-free transforms). |
| "Bitwise reproducibility as an incomplete correctness signal" (structural audit, climate model) | EarthArXiv preprint 2026 | eartharxiv.org/repository/view/13757/ | none | outside | Shows thread-count identity checks are blind to thread-invariant state leaks: identity is not correctness, the oracle must stay. |
| **Testing graph engines** | | | | | |
| Mang, Ba, He, Rigger: Graph-cutting (GSlicer) | SIGMOD 2025 (PACMMOD 3(3)) | doi.org/10.1145/3725300 | none | outside | Metamorphic testing of graph *algorithms* (NetworkX, Neo4j, Kùzu): 39 new bugs, 34 fixed; > 100 NetworkX APIs covered by a few relations. Applies to our kernels and our NetworkX backend (D20). |
| GraphFuzz: differential fuzzing of graph algorithm implementations | arXiv 2025 | arxiv.org/abs/2502.15160 | none | outside | Differential NetworkX vs igraph: 12 new bugs. Our oracle (#5) is differential; this says how to generate inputs. |
| Testing graph database engines via query partitioning (GDBMeter) | ISSTA 2023 | doi.org/10.1145/3597926.3598044 | ingest-query | outside (testing not in scope) | Query-partitioning oracle; for m9-03 GQL subset and the traversal API. |
| GDsmith: detecting bugs in Cypher graph database engines | ISSTA 2023 | doi.org/10.1145/3597926.3598046 | ingest-query | outside | Differential Cypher testing. |
| Grand: finding bugs in Gremlin-based GDBs via randomized differential testing | ISSTA 2022 | doi.org/10.1145/3533767.3534409 | ingest-query | outside | Same family. |
| Testing graph database systems via graph-aware metamorphic relations | PVLDB 2023 | doi.org/10.14778/3636218.3636236 | ingest-query | outside | Metamorphic relations on the graph itself, not the query. |
| Pillai et al.: All file systems are not created equal (ALICE, BOB) | OSDI 2014 | usenix.org/conference/osdi14/technical-sessions/presentation/pillai | io-memory | outside (crash consistency not in scope) | Append-then-publish protocols break on real file systems; ADR 0007 promises crash-injection tests (#28) — this is the method. |
| FormatFuzzer: effective fuzzing of binary file formats | ACM TOSEM 2023 | doi.org/10.1145/3628157 | ingest-query | outside | Structure-aware fuzzing of binary formats (`.eg`, PMTiles payload). |
| Fifield: A better zip bomb | USENIX WOOT 2019 | usenix.org/conference/woot19/presentation/fifield | ingest-query | outside | 10 MB → 281 TB with one DEFLATE layer. Our ingest decompresses gzip/zstd inputs and the viewer decompresses tiles: a budget must bound *output*, not input. |
| **Benchmarking and measurement** | | | | | |
| Hoefler, Belli: Scientific benchmarking of parallel computing systems | SC 2015 | doi.org/10.1145/2807591.2807644 | none | outside | The 12 rules (CIs, medians, speedup bases). Storage cites Hoefler once; nobody owns methodology. |
| Mytkowicz et al.: Producing wrong data without doing anything obviously wrong | ASPLOS 2009 | doi.org/10.1145/1508244.1508275 | none | outside | Environment size and link order flip conclusions; setup randomization. Our A/B rule (CLAUDE.md § 6) lacks it. |
| Kalibera, Jones: Rigorous benchmarking in reasonable time | ISMM 2013 | doi.org/10.1145/2464157.2464160 | none | outside | How many repetitions at which level (build / process / iteration); effect-size CIs. |
| Laaber, Scheuner, Leitner: Software microbenchmarking in the cloud | EMSE 2019 | doi.org/10.1007/s10664-019-09681-1 | none | outside | Interleaved A/B in one run on noisy hosts: our "shared machine" situation. |
| Mehrotra, Anand, Margo, Hajidehi, Seltzer: SoK: the faults in our graph benchmarks | arXiv 2024 | arxiv.org/abs/2404.00766 | none | outside | 12-year review; isolated vertices change BFS time 10×; degree ordering 40% on twitter-2010. Relevant to how we report numbers vs NetworKit/GVE. |
| Meng et al.: Revisiting graph analytics benchmark | SIGMOD 2025 | doi.org/10.1145/3725345 | none | outside | New generator (scale, density, diameter) and an LLM-based API-usability score: a template for m8-03 public benchmarks and for usability claims. |
| On the external validity of average-case analyses of graph algorithms | ESA 2022 / TALG | doi.org/10.1145/3633778 | none | outside | Which generators predict behaviour on real graphs; for m1-10 graph families and m0-04 datasets. |
| Wheatman et al.: BYO, benchmarking graph containers | PVLDB 2024 | doi.org/10.14778/3665844.3665859 | storage | covered (storage names it) | Apples-to-apples method. |
| Haas, Lee, Bonnet, Leis: SSD-iq | PVLDB 2025 | doi.org/10.14778/3749646.3749694 | io-memory | not checkable; likely missed (device behaviour not in the brief) | Nine SSDs with similar specs differ widely in write amplification and latency under load. ADR 0026 rests on one drive's burst/steady-state behaviour. |
| Lee, Ziegler, Leis: How to write to SSDs | PVLDB 2026 | doi.org/10.14778/3801059.3801063 | io-memory | not checkable | Write patterns with WAF = 1 on commodity SSDs; we write tens of GB per ingest and per spill. |
| Maneas et al.: SSD field study (WAF, read/write ratios) | FAST 2022 | usenix.org/system/files/fast22-maneas.pdf | io-memory | outside | WAF in the field; endurance budgeting for repeated ingests/spills on laptops. |
| **Graphs that change** | | | | | |
| Aspen: low-latency graph streaming using compressed purely-functional trees | PLDI 2019 | doi.org/10.1145/3314221.3314598 | storage | covered (storage) | Compressed functional trees for streaming graphs. |
| Fuchs, Margan, Giceva: Sortledton | PVLDB 2022 | doi.org/10.14778/3514061.3514065 | storage | missed? | Transactional dynamic graph structure at ~2.1× CSR memory (abstract). |
| Su et al.: Revisiting the design of in-memory dynamic graph storage | SIGMOD 2025 | doi.org/10.1145/3709720 | storage | missed? | Systematic comparison: Aspen 3.3–10.8× CSR memory; large read gap vs static CSR remains — supports our "static canonical + segments" plan (m9-04). |
| LSGraph: a locality-centric high-performance streaming graph engine | EuroSys 2024 | doi.org/10.1145/3627703.3650076 | storage | missed? | Streaming engine with learned index. |
| LSMGraph (multi-level CSR); Spruce | SIGMOD 2024 | doi.org/10.1145/3698818 ; doi.org/10.1145/3639282 | storage | missed? | LSM of CSRs — the closest published design to our time-partitioned segments. |
| PaC-trees: parallel and compressed purely-functional collections (CPAM) | PLDI 2022 | doi.org/10.1145/3519939.3523733 | storage | missed? | Compressed parallel functional collections. |
| CommonGraph: graph analytics on evolving data | ASPLOS 2023 | doi.org/10.1145/3575693.3575713 | none | outside | Evaluating a query over many snapshots by sharing the common graph — our `window()` over many windows. |
| Sahu: DF Leiden (dynamic frontier) and heuristic dynamic Leiden | arXiv 2024 | arxiv.org/abs/2405.11658 ; arxiv.org/abs/2410.15451 | communities | missed? | Incremental Leiden on GVE-Leiden; 6.1× over static on batch updates (64 cores, abstract). Map updates after an append. |
| Maintaining Leiden communities in large dynamic graphs | arXiv 2026 | doi.org/10.48550/arxiv.2601.08554 | communities | missed? | Newer incremental Leiden. |
| Parallel batch-dynamic algorithms for k-core decomposition | SPAA 2022 | doi.org/10.1145/3490148.3538569 | kernels | not checkable | Batch-dynamic kernels. |
| Steer et al.: Raphtory, temporal graph engine for Rust and Python | JOSS 2024 | doi.org/10.21105/joss.05940 | ingest-query | covered (ingest-query) | Direct competitor on temporal graphs. |
| **Approximation under a budget** | | | | | |
| Yang et al.: Oasis, out-of-core approximate graph system via all-distances sketches | FAST 2025 | usenix.org/conference/fast25/presentation/yang | io-memory | not checkable; likely missed | Out-of-core ADS construction and estimators: the competitor to HyperBall (m3-09) under a budget. |
| Cohen: All-distances sketches, revisited (HIP estimators) | PODS 2014 | doi.org/10.1145/2594538.2594546 | kernels | not checkable | Lower-variance estimator than HyperLogLog counters for neighbourhood functions. |
| Cousins, Wohlgemuth, Riondato: Bavarian | KDD 2021 / TKDD 2023 | doi.org/10.1145/3447548.3467354 | kernels | not checkable | Betweenness by progressive sampling with guarantees; for m3-09. |
| Pellegrina, Vandin: SILVAN | arXiv 2021 (KDD) | arxiv.org/abs/2106.03462 | kernels | not checkable | Same, non-uniform bounds. |
| Demystifying graph sparsification algorithms in graph properties preservation | PVLDB 2023 | doi.org/10.14778/3632093.3632106 | maps / none | missed? (maps names sparsification for bundling only) | Which sparsifiers keep which properties — for drawing edges at deep zoom (§ 5 item 3) and for budgeted approximate kernels. |
| **Browser and Rust practice** | | | | | |
| Jangda, Powers, Berger, Guha: Not so fast (WebAssembly vs native) | USENIX ATC 2019 | usenix.org/conference/atc19/presentation/jangda | none | outside | Causes of the wasm gap (bounds/stack checks, register pressure). Our lab measured our own gap; this explains it. |
| Sengupta et al.: From WebGL to WebGPU, a reality check | IMC 2025 | doi.org/10.1145/3730567.3764504 | maps | missed? | WebGPU loses to CPU on small inputs (setup/sync), wins on large: for m9-05 GPU layout and the viewer. |
| Abdi, Posluns, Zhang, Wang, Jeffrey: When is parallelism fearless and zero-cost with Rust? | SPAA 2024 | doi.org/10.1145/3626183.3659966 | kernels | not checkable | Rayon is fearless for regular parallelism only; irregular kernels need `unsafe` or costly checks — exactly our edge-map and Leiden. |
| **Human factors** | | | | | |
| Zong et al.: Rich screen reader experiences for accessible data visualization | EuroVis 2022 (CGF) | doi.org/10.1111/cgf.14519 | maps | missed? (maps mentions accessibility only as colour) | Structure-navigable charts for screen readers; m5-11 promises accessibility. |
| Zhao, Nacenta, Sukhai, Somanath: TADA, node-link diagrams for blind and low-vision people | CHI 2024 | doi.org/10.1145/3613904.3642222 | maps | missed? | The only BLV study on node-link diagrams found; touch + audio. |
| Understanding barriers to network exploration with visualization: a report from the trenches | IEEE VIS 2022 (TVCG) | doi.org/10.1109/TVCG.2022.3209487 | maps | missed? | Interview study of practitioners: what blocks network exploration. User research our "analyst" persona lacks. |
| Knowledge graphs in practice: characterizing their users, challenges, and visualization opportunities | IEEE VIS 2023 (TVCG) | doi.org/10.1109/TVCG.2023.3326904 | maps | missed? | Same, for KG practitioners. |
| Evaluating graph layout algorithms: a systematic review of methods and best practices | CGF 2024 | doi.org/10.1111/cgf.15073 | maps | missed? | How layout claims should be evaluated (our D17 evidence is two metrics). |
| **Privacy and AI agents** | | | | | |
| From local to global: a Graph RAG approach | arXiv 2024 | doi.org/10.48550/arxiv.2404.16130 | communities | communities names GraphRAG once | GraphRAG summarizes a *hierarchical Leiden* partition for LLMs: our map hierarchy is the same object, for the MCP server (m9-02). |
| In-depth analysis of graph-based RAG in a unified framework | PVLDB 2025 | arxiv.org/abs/2503.04338 | none | outside | Which graph-RAG designs work. |
| Local edge-DP graph algorithms; DP release of count-weighted graphs | PVLDB 2025; SIGMOD 2023 | doi.org/10.14778/3749646.3749687 ; doi.org/10.1145/3589299 | none | outside | A shared static map file exposes labels and edges; DP and aggregation thresholds are the known tools. |
| **Inside a scope, likely missed** | | | | | |
| GraphCSR: degree-equalized CSR | PVLDB 2025 | doi.org/10.14778/3749646.3749691 | storage | missed? | Load balance of CSR scans. |
| "Improving graph compression for efficient resource-constrained graph analytics" | PVLDB 2024 | doi.org/10.14778/3665844.3665852 | storage | missed? | Exactly our constraint (compression under a memory budget). |
| Frontier-guided graph reordering | PPoPP 2025 | doi.org/10.1145/3710848.3710895 | storage | missed? | Ordering for traversal kernels. |
| Rebo: locality-aware graph processing via reordering and blocking | IPDPS 2026 | doi.org/10.1109/IPDPS65963.2026.00088 | storage | missed? | Ordering + blocking. |
| Hiding page fault latencies in graph processing applications that cannot fit in memory | IPDPS 2026 | doi.org/10.1109/IPDPS65963.2026.00089 | io-memory | not checkable | The layout's 733 GB of mmap re-reads (§ 5). |
| Practically tackling memory bottlenecks of graph-processing workloads | IPDPS 2024 | doi.org/10.1109/IPDPS57955.2024.00096 | kernels | not checkable | § 5 item 5, memory-bandwidth ceiling. |
| CoroGraph: cache efficiency vs work efficiency | PVLDB 2023 | doi.org/10.14778/3636218.3636240 | kernels | not checkable | Same. |
| CAVE: concurrency-aware graph processing on SSDs | SIGMOD 2024 | doi.org/10.1145/3654928 | io-memory | not checkable | SSD queue depth for graph I/O. |
| Graphago: SSD-based graph processing via activity-aware preprocessing | SC 2025 | doi.org/10.1145/3712285.3759851 | io-memory | not checkable | Recent out-of-core system, not on the other verifier's list. |
| A survey of large-scale out-of-core graph processing | ACM CSUR 2026 | doi.org/10.1145/3838595 | io-memory | not checkable | Recent survey; check against the other verifier's list. |
| GBBS lineage: PASGAL; Parallel k-core decomposition: theory and practice; ParlayANN (deterministic) | SPAA 2024; SIGMOD 2025; PPoPP 2024 | doi.org/10.1145/3626183.3660258 ; doi.org/10.1145/3725332 ; doi.org/10.1145/3627535.3638475 | kernels | not checkable | Newest GBBS-line kernels and a deterministic parallel design. |
| Implementation strategies for views over property graphs | SIGMOD 2024 | doi.org/10.1145/3654949 | ingest-query | missed? | D10 mask vs materialize, in a GDBMS. |
| MiniClean: a single-machine system for cleaning big graphs | SIGMOD 2025 | doi.org/10.1145/3722212.3725115 | ingest-query | missed? | Data quality at ingest (duplicates, conflicts) on one machine. |
| Graph Tiles (poster) | GD 2025 | doi.org/10.4230/LIPIcs.GD.2025.51 | maps | missed? | Same name as our idea; check novelty claims in maps § 5. |
| LargeNetVis: large temporal networks via community taxonomies | IEEE VIS 2022 (TVCG) | doi.org/10.1109/TVCG.2022.3209477 | maps | missed? | Community-driven overview of large temporal networks. |
| FastEnsemble: scalable ensemble clustering | PLOS Complex Systems 2025 | doi.org/10.1371/journal.pcsy.0000069 | communities | missed? | Consensus across seeds = the uncertainty measure in m9-10. |

## (c) Missing topics, ranked by relevance to Everygraph

1. **Numerical and cross-platform determinism** (promise 3, D14; owner today: nobody — kernels
   covers deterministic *algorithms*, not arithmetic, RNG or libm). Anchors: Demmel–Nguyen
   ARITH 2013 and TC 2015; Ahrens et al. TOMS 2020; Müller et al. ICDE 2018; FPRev ATC 2025;
   CORE-MATH ARITH 2022; Philox SC 2011; Wasm 3.0 deterministic profile.
   *Question:* can we drop the restriction in m4-04 that identity across budgets holds only
   for integer weights (binned or exact accumulators for f64 weight sums), at what cost
   against fixed-block reduction order; which transcendental functions may enter results
   (layout, PageRank variants), and is a correctly rounded libm the way; what RNG policy
   makes every randomized step independent of thread count and regime?
2. **Testing and verifying a graph engine** (no-OOM and correctness promises; m0-05 oracle,
   m3-10 budget fuzzing, m3-11, #28 crash injection, m2-03 fuzzing). Anchors: Graph-cutting
   SIGMOD 2025; GraphFuzz 2025; GDBMeter ISSTA 2023; Pillai et al. OSDI 2014; FormatFuzzer
   TOSEM 2023; Fifield WOOT 2019. *Question:* which oracles (differential vs NetworkX/igraph,
   metamorphic relations on the graph, property-based) find the most bugs per CI minute
   for kernels and the NetworkX backend; how to test crash consistency of `.eg`
   append/`compact` on ext4, xfs, btrfs, APFS; how to bound decompression output in
   ingest and the viewer?
3. **Measurement methodology on workstations and laptops** (promise 1 and every number
   in DESIGN; CLAUDE.md § 3 rules; m3-12 perf tracking, m8-03 public benchmarks). Anchors:
   Hoefler–Belli SC 2015; Mytkowicz ASPLOS 2009; Kalibera–Jones ISMM 2013; Laaber et al. EMSE
   2019; SoK graph benchmarks 2024; Revisiting graph analytics benchmark SIGMOD 2025;
   SSD-iq PVLDB 2025. *Question:* how many repetitions at which level, how to report CIs,
   how to randomize setup; how far our "idle machine" numbers generalize to other drives
   (the SLC burst behind ADR 0026) and to laptops with thermal limits; how to publish
   comparisons with NetworKit/GVE that survive the SoK's faults (isolated vertices,
   ordering, preprocessing excluded or not)?
4. **Graphs that change** (m7-05 temporal windows, m9-04 segments, incremental append,
   map update after append; competitor Raphtory). Anchors: Revisiting dynamic graph storage
   SIGMOD 2025; LSMGraph SIGMOD 2024; Sortledton PVLDB 2022; CommonGraph ASPLOS 2023;
   DF Leiden 2024; batch-dynamic k-core SPAA 2022. *Question:* are time-partitioned
   immutable segments over a static compressed canonical adjacency the right design
   (vs LSM-of-CSR or PaC-trees), what does a deterministic incremental Leiden and a stable
   map update look like, and what must the `.eg` format reserve now so that appends do not
   need a format break?
5. **Approximation and sketching under a budget** (m3-09 centrality; labelled approximate
   variants per DECISIONS). Anchors: Oasis FAST 2025; Cohen PODS 2014; Bavarian KDD 2021;
   SILVAN; Demystifying sparsification PVLDB 2023. *Question:* HyperBall vs ADS/HIP
   under a memory budget, bit-stable across regimes; which sampling estimators can be made
   deterministic for any thread count. Partly in the earlier survey (HyperBall); the
   rest fits a kernels follow-up rather than a new brief.
6. **Human factors: accessibility, API usability, user research** (promise 3 "see it";
   m5-11 perf/accessibility, m6-08 tutorials, the analyst persona). Anchors: Zong et al. CGF
   2022; TADA CHI 2024; Barriers to network exploration, VIS 2022; KGs in practice VIS 2023; Revisiting graph
   analytics benchmark (API usability). *Question:* what a non-visual route through a
   hierarchical map looks like (the hierarchy is a natural tree to navigate), and what
   evidence we need before claiming usability.
7. **Storage device wear and write volume** (we write tens of GB per Friendster ingest and
   per spill — ADR 0026 cites a 45–55 GB ingest; consumer drives are rated in the low hundreds of TBW per TB, per vendor and retail sources, not peer-reviewed). Anchors: SSD-iq; How to write to
   SSDs; Maneas FAST 2022; "Wear leveling considered harmful" HotStorage 2022
   (doi.org/10.1145/3538643.3539750). *Question:* how many full-Friendster ingest+spill
   cycles a typical laptop SSD tolerates, and whether the planner should prefer recompute
   over spill when the drive is consumer-grade. Fold into topic 3.
8. **Browser compute** (D22, m9-01, m9-05). Anchors: Jangda ATC 2019; Yan et al. IMC 2021
   (doi.org/10.1145/3487552.3487827); IMC 2025 WebGL→WebGPU; Wasm 3.0. Our lab already
   measured most of it (memory64 ~3× slower, ceiling 265 M edges); low priority.
9. **Rust parallel practice** (D1, D12, D13). Anchors: SPAA 2024 fearless parallelism;
   ASE 2022 runtime performance of Rust (doi.org/10.1145/3551349.3559494). Fits the kernels
   area; flag to its author rather than a new brief.
10. **Privacy of shared maps** (analysts share one static file of a sensitive graph).
    Anchors: local edge-DP algorithms PVLDB 2025; DP release of count-weighted graphs
    SIGMOD 2023. *Question:* do we need a "redacted export" (labels off, small-count
    suppression) before an investigator shares a map? Low effort, low priority.
11. **Graphs for LLM agents** (MCP after v0.1). Anchors: GraphRAG 2024; graph-RAG analysis
    PVLDB 2025. Our hierarchy with exact counts is what GraphRAG builds ad hoc; a note for
    m9-02, not research now.
12. **Out of scope, say so:** GNN training and data loading (DESIGN § 2: not a GNN
    framework; the snowball is full of them — MariusGNN, Ginex, DiskGNN — ignore); graph
    vector-search indexes (DiskANN family, Starling) except as I/O-layout ideas; energy
    measurement (thin literature, nothing actionable found).

## (d) Proposed second wave (four briefs)

**W1. Bit-identical numerics across threads, budgets and platforms.**
Scope: reproducible summation (binned, exact/long accumulators, fixed-tree reductions) and
its cost; FMA and compiler contraction in Rust/LLVM; correctly rounded libm and which
functions we need; counter-based RNG and seeding policy; wasm relaxed SIMD and NaN bits;
tools that check accumulation order (FPRev). Key questions: drop the integer-weight
restriction of m4-04?; reproducible f64 weight sums in Leiden aggregation at what slowdown
on Friendster-sized level-1 graphs (paper numbers only); a written rule list for m3-11 CI.
Anchors: Demmel–Nguyen 2013/2015, Ahrens et al. 2020, Müller et al. 2018, FPRev 2025,
CORE-MATH 2022, Salmon et al. 2011, Wasm 3.0 spec, Boldo et al. 2023.

**W2. Testing a graph engine: oracles, fuzzing, crash consistency, budgets.**
Scope: differential and metamorphic testing of graph algorithms and of the NetworkX
backend; property-based generation of graphs; fuzzing the text/Parquet readers, the `.eg`
decoder and tile decoder (structure-aware; decompression bombs); crash-consistency testing
of append-and-publish files; fault injection (ENOSPC, cgroup memory limits) for the no-OOM
promise; mutation testing as test-quality evidence. Key questions: which relations to
adopt from Graph-cutting for our kernels; how ALICE-style testing applies to `.eg`; how to
make budget fuzzing (m3-10) find the O(n)-state OOM cases. Anchors: Graph-cutting SIGMOD
2025, GraphFuzz 2025, GDBMeter/GDsmith/Grand, Pillai et al. OSDI 2014, FormatFuzzer 2023,
Fifield 2019.

**W3. Measuring honestly: statistics, noise, drives and public comparisons.**
Scope: repetition and CI methodology; measurement bias and setup randomization; interleaved
A/B on shared hosts; CPU frequency, SMT and thermal effects on a 16-core desktop and on
laptops; SSD burst vs steady state, write amplification and wear (our ingest and spill
volumes); graph-benchmark faults and dataset/generator validity; how to design m8-03 public
benchmarks. Key questions: does the harness's "median of 6 / of 3" meet Kalibera–Jones; how
should ADR 0026's drive-bound rate be stated for other drives; what comparison protocol
against NetworKit/GVE/DuckDB survives the SoK. Anchors: Hoefler–Belli 2015, Mytkowicz 2009,
Kalibera–Jones 2013, Laaber 2019, SoK graph benchmarks 2024, Revisiting graph analytics
benchmark 2025, external validity of average-case analyses, ESA 2022, SSD-iq 2025, How to write to SSDs 2026.

**W4. Graphs that change: segments, incremental kernels, stable maps.**
Scope: dynamic graph storage (functional trees, PMA/Terrace, Sortledton, LSM-of-CSR) vs our
plan of immutable time-partitioned segments; multi-snapshot evaluation (CommonGraph);
batch-dynamic kernels (CC, PageRank, k-core) and incremental Leiden; dynamic graph drawing
and map stability after an append; temporal semantics vs Raphtory. Key questions: what the
`.eg` format must reserve now (m2-01) to allow appends; can incremental Leiden be
deterministic and budget-independent; how much a map may move after an append. Anchors:
Revisiting dynamic graph storage SIGMOD 2025, LSMGraph SIGMOD 2024, Sortledton PVLDB 2022, PaC-trees PLDI 2022,
CommonGraph ASPLOS 2023, DF Leiden 2024, batch-dynamic k-core SPAA 2022, Raphtory JOSS 2024.

*Reserve, if a fifth slot opens:* human factors (topic 6). Otherwise send its anchors to
the maps author as a follow-up question, and send topic 5 and 9 anchors to the kernels
author.
