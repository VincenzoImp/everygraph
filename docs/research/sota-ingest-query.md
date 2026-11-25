# State of the art: ingest, identifiers, format, properties, views, queries and API

Survey of 2026-11-25 for the area "ingest-query" (web and paper research, Opus; brief
`sota-brief.md`). It covers getting data in (parsing, decompression, CSR build), the id
dictionary (D8), the `.eg` container (D7), property columns (D9), views (D10), the Python
and NetworkX surface (D19, D20), the traversal API (D21), promise 1 (ADR 0026), and the
engines we are compared with. It builds on `docs/research/memory-bounded-algorithms.md`
and does not cover the out-of-core systems another survey is verifying.

**How the reading was done.** Four readers: this report's author (engines, views, API,
budgeted queries) and three delegated readers with narrow briefs — (A) minimal perfect
hashing, (B) parsing, decompression, sorting and CSR build, (C) formats and containers. The
column "by" in § 1 says who read each work. Their notes were checked against the text
files at random (PIGO's 3.5 s, PHastR's Table 3, IPS4o's 8.37×, Lance's alignment remark,
csveee's 186 / 40 GB/s: all as quoted). Nothing was built or measured; every figure is the
paper's, on the paper's machine, and **not comparable with ours as is**. Where a line says
"estimate" it is an extrapolation by us, not a measurement.

PDFs: `/home/tux/graphlab/papers/ingest-query/` (text in `txt/`).

---

## 1. Scope and reading log

Read levels: **full** = method and evaluation read; **partial** = named sections; **abs** =
abstract and results only; **docs** = specification or documentation page; **n/a** = not
accessible. By: S = this report, A/B/C = delegated readers.

| Work | Venue, year | Link (verified) | Level | By | PDF |
|---|---|---|---|---|---|
| **Identifiers / MPHF** | | | | | |
| PtrHash (Groot Koerkamp) | SEA 2025 | arxiv.org/abs/2502.15539 | full + app. A, B | A | 2025-grootkoerkamp-ptrhash.pdf |
| PHast / PHast+ (Beling, Sanders) | arXiv v6, 2025 | arxiv.org/abs/2504.17918 | full | A | 2025-beling-phast.pdf |
| PHastR (Vigna) | arXiv v4, Nov 2026 | arxiv.org/abs/2610.17833 | full (§ 3–8) | A | 2026-vigna-phastr.pdf |
| Modern MPHF survey (Lehmann, Mueller, Pagh, Pibiri, Sanders, Vigna, Walzer) | arXiv 2025/26 | arxiv.org/abs/2506.06536 | partial (§ 6.1, 9–10, app. C, E) | A | 2025-lehmann-mphf-survey.pdf |
| PHOBIC (Hermann et al.) | ESA 2024 | arxiv.org/abs/2404.18497 | full | A | 2024-hermann-phobic.pdf |
| PTHash parallel + external memory (Pibiri, Trani) | TKDE 2024 | arxiv.org/abs/2106.02350 | partial (§ 4–5) | A | 2023-pibiri-pthash-parallel-em.pdf |
| Consensus-RecSplit (Lehmann et al.) | arXiv 2025 | arxiv.org/abs/2502.05613 | partial | A | 2025-lehmann-consensus-recsplit.pdf |
| Non-minimal k-perfect hashing (Groot Koerkamp et al.) | ESA 2026 | arxiv.org/abs/2607.07257 | abs + § 4 | A | 2026-grootkoerkamp-nonminimal-kperfect.pdf |
| PTHash; RecSplit; ShockHash; bipartite ShockHash; SicHash; BBHash; GPU RecSplit | 2017–2024 | arXiv 2104.10402, 1910.06416, 2308.09561, 2310.14959, 2210.01560, 1702.03154, 2212.09562 | abs | A | 2021-pibiri-pthash, 2020-esposito-recsplit, 2023-lehmann-shockhash, 2024-lehmann-bipartite-shockhash, 2023-lehmann-sichash, 2017-limasset-bbhash, 2023-bez-gpu-recsplit |
| FMPH / FMPHGO (Beling) | ACM JEA 2023 | doi 10.1145/3596453 | n/a (403, no preprint); numbers only via PHast and survey tables | A | – |
| Crates `ph` 0.11, `sux` 0.15.4, `ptr_hash` 2.1.5 | crates.io | source tarballs | source read | A | – |
| **Parsing, decompression** | | | | | |
| csveee, "One Pass to Parse Them All" (Ellmann, Neumann) | PVLDB 19, 2026 | vldb.org/pvldb/vol19/p3579-ellmann.pdf | full | B | 2026-ellmann-csveee.pdf |
| simdjson (Langdale, Lemire) | VLDB J. 2019 | arxiv.org/abs/1902.08318 | full (method, evaluation) | B | 2019-langdale-simdjson.pdf |
| ParPaRaw (Stehle, Jacobsen) | PVLDB 13, 2020 | arxiv.org/abs/1905.13415 | full | B | 2020-stehle-parparaw.pdf |
| Instant Loading (Mühlbauer et al.) | PVLDB 6, 2013 | vldb.org/pvldb/vol6/p1702-muehlbauer.pdf | abs | B | 2013-muehlbauer-instantloading.pdf |
| Speculative CSV parsing (Ge et al.) | SIGMOD 2019 | microsoft.com/…/chunker-sigmod19.pdf | partial (§ 3–4.4) | B | 2019-ge-speculativecsv.pdf |
| rapidgzip (Knespel, Brunst) | HPDC 2023 | arxiv.org/abs/2308.08955 | partial (§ 4.4–4.9) | B | 2023-knespel-rapidgzip.pdf |
| DuckDB CSV rate | DuckDB blog 2025-09-08 | duckdb.org/2025/09/08/… | docs | B | – |
| **Graph loading, CSR build, sorting** | | | | | |
| PIGO (Gabert, Çatalyürek) | IPDPSW 2021 | graphanalysis.org/GrAPL2021/PIGO_….pdf | full (4 pages; figures unreadable as text) | B | 2021-gabert-pigo.pdf |
| GVEL (Sahu) | Euro-Par W 2024 | arxiv.org/abs/2311.14650 | full | B | 2023-sahu-gvel.pdf |
| ParaGrapher (Koohi Esfahani et al.) | arXiv 2024 | arxiv.org/abs/2404.19735 | full | B | 2024-koohiesfahani-paragrapher.pdf |
| ParaGrapher follow-up (PG-Fuse, CompBin) | arXiv 2025 | arxiv.org/abs/2507.00716 | full | B | 2025-koohiesfahani-paragrapher-accel.pdf |
| IPS4o / IPS2Ra (Axtmann, Witt, Ferizovic, Sanders) | TOPC 2022 | arxiv.org/abs/2009.13569 | partial (§ 4.1, 7, 8) | B | 2022-axtmann-ips4o.pdf |
| DovetailSort (Dong, Dhulipala, Gu, Sun) | PPoPP 2024 | arxiv.org/abs/2401.00710 | partial (§ 1–3, 6.1–6.2) | B | 2024-dong-integersort.pdf |
| Semisort (Dong et al.) | SPAA 2023 | arxiv.org/abs/2304.10078 | partial (tables 3–5) | B | 2023-dong-semisort.pdf |
| GAP benchmark (Beamer et al.); GBBS (Dhulipala et al.) | 2015; SPAA 2018 | arXiv 1508.03619, 1805.05208 | abs (no load numbers in either) | B | 2015-beamer-gap.pdf, 2018-dhulipala-gbbs.pdf |
| RADULS2 (Kokot et al.); parallel semisort (Gu et al.) | 2017; SPAA 2015 | arXiv 1703.00687; cs.cmu.edu/~jshun/semisort.pdf | abs | B | 2017-kokot-raduls2.pdf, 2015-gu-semisort.pdf |
| Then et al., parallel graph loading | GRADES 2016 | doi 10.1145/2960414.2960418 | n/a | B | – |
| Ringo (Perez et al.) | SIGMOD 2015 demo | cs.stanford.edu/people/jure/pubs/ringo-sigmod15.pdf | full | S | 2015-perez-ringo.pdf |
| **Formats and containers** | | | | | |
| GraphAr (Li et al.) | PVLDB 2025 | arxiv.org/abs/2312.09577 | full | C | 2025-li-graphar.pdf |
| Lance structural encodings (Pace et al.) | arXiv 2025 | arxiv.org/abs/2504.15247 | full | C | 2025-pace-lance.pdf |
| Empirical evaluation of columnar formats (Zeng et al.) | PVLDB 2024 | arxiv.org/abs/2304.05028 | full | C | 2024-zeng-columnar-eval.pdf |
| F3 (Zeng et al.) | SIGMOD 2025 | db.cs.cmu.edu/papers/2025/zeng-sigmod2025.pdf | full | C | 2025-zeng-f3.pdf |
| AnyBlox (Gienieczko et al.) | PVLDB 18, 2025 | vldb.org/pvldb/vol18/p4017-gienieczko.pdf | full | C | 2025-gienieczko-anyblox.pdf |
| BtrBlocks (Kuschewski et al.) | SIGMOD 2023 | cs.cit.tum.de/…/btrblocks.pdf | full | C | 2023-kuschewski-btrblocks.pdf |
| FastLanes (Afroozeh, Boncz) | PVLDB 16, 2023 | vldb.org/pvldb/vol16/p2132-afroozeh.pdf | abs | C | 2023-afroozeh-fastlanes.pdf |
| Vortex; Arrow IPC; Parquet page index; Nimble; Lance file; GraphAr spec and release state; PMTiles v3; graph-tool `.gt`; NetworKit nkbg; webgraph-rs | specs, 2023–2026 | docs.vortex.dev; arrow.apache.org/docs/format; parquet.apache.org; github.com/facebookincubator/nimble; graphar.apache.org; github.com/protomaps/PMTiles | docs | C | – |
| Cologni & Vigna, differential graph compression | WAW 2026 | doi 10.1007/978-3-032-27193-8_7 | n/a (paywall) | C | – |
| Content-addressed formats (Git packs, OCI, IPLD/CAR, casync); HDF5 | – | – | background knowledge, not re-read | C | – |
| **Engines, views, queries, API** | | | | | |
| Kùzu (Feng, Jin et al.) | CIDR 2023 | cidrdb.org/cidr2023/papers/p48-jin.pdf | full | S | 2023-jin-kuzu.pdf |
| DuckPGQ (ten Wolde et al.) | CIDR 2023 | cidrdb.org/cidr2023/papers/p66-wolde.pdf | full | S | 2023-tenwolde-duckpgq.pdf |
| Graphsurge (Sahu, Salihoglu) | SIGMOD 2021 | arxiv.org/abs/2004.05297 | partial (§ 1, 5, 6.1–6.3) | S | 2021-sahu-graphsurge.pdf |
| Kaskade (da Trindade et al.) | ICDE 2020 | arxiv.org/abs/1906.05162 | abs + method skim | S | 2020-trindade-kaskade.pdf |
| Querying big graphs within bounded resources (Fan, Wang, Wu) | SIGMOD 2014 | homepages.inf.ed.ac.uk/wenfei/papers/sigmod14.pdf | partial (§ 1–2, 6 setup and claims) | S | 2014-fan-boundedresources.pdf |
| MS-BFS, "The More the Merrier" (Then et al.) | PVLDB 8, 2014 | vldb.org/pvldb/vol8/p449-then.pdf | abs | S | 2014-then-msbfs.pdf |
| Raphtory (Steer et al.) | arXiv 2023 / JOSS | arxiv.org/abs/2306.16309 | full (short paper) + repo README | S | 2023-steer-raphtory.pdf |
| NetworKit (Staudt, Sazonovs, Meyerhenke) | Network Science 2016 | arxiv.org/abs/1403.3005 | partial (§ 6.2 profiles, § 8 evaluation) | S | 2016-staudt-networkit.pdf |
| GraphScope Flex (He et al.) | SIGMOD 2024 companion | arxiv.org/abs/2312.12107 | abs + § 4 (GRIN, GraphAr) | S | 2024-he-graphscopeflex.pdf |
| Binary + worst-case-optimal joins (Mhedhbi, Salihoglu) | PVLDB 2019 | arxiv.org/abs/1903.02076 | abs | S | 2019-mhedhbi-wcojbinary.pdf |
| DuckDB USING KEY (Bamberg, Hirn, Grust) | SIGMOD 2025 demo | doi 10.1145/3722212.3725107 | n/a (host unreachable); DuckDB blog post read instead | S | – |
| NetworkX backend dispatching (3.7 docs; 3.5, 3.6 release notes; source of `read_edgelist`, `from_pandas_edgelist`) | 2025–2026 | networkx.org/documentation/stable/reference/backends.html | docs + source | S | – |
| nx-cugraph README | 2026 | github.com/rapidsai/nx-cugraph | docs | S | – |
| Neo4j GDS: memory estimation; `gds.graph.filter` | current docs | neo4j.com/docs/graph-data-science/current/… | docs | S | – |
| graph-tool quick start (GraphView, filters) | current docs | graph-tool.skewed.de | docs | S | – |
| LadybugDB, Grafeo, DuckPGQ extension repositories | 2026 | github.com/LadybugDB/ladybug, GrafeoDB/grafeo, cwida/duckpgq-extension | docs | S | – |

---

## 2. The state of the art, as a map

### 2.1 Minimal perfect hashing (D8)

Two families lead in 2025–26: **PtrHash** (pilots per bucket, fixed 8-bit, Cuckoo-style
eviction) and **PHast → PHast+ → PHastR** (bucket placement into a local slice, bumping,
bit-parallel seed tests). Everything else (PHOBIC, PTHash-HEM, FMPH, SicHash, BBHash,
RecSplit, ShockHash, Consensus) is smaller or older but **2–10× slower to query**.

| Method | bits/key | build ns/key (1 t / many t) | query ns | Conditions |
|---|---|---|---|---|
| PtrHash default λ=3.5 cubic, Vec remap | 2.29 + 0.33 = 2.62 | — / ~19 at 6 t | 20.0 loop, 8.6 streamed | i7-10750H 6 c, 10^9 u64 keys (PtrHash Table 1) |
| PtrHash default, strings | 2.40 | 32 at 6 t | 37 (23 streamed) | same machine, 3·10^8 strings 10–50 B (Table 2) |
| PHast+ δ=2, S=8 | 2.07 | 98 / 22 at 12 t | 26 | Ryzen 5600G 6c/12t, 5·10^8 strings (PHast Table 5.3); PtrHash on the same row: 2.40, 100 / 20, 32 |
| **PHastR** S=8, λ=4.25 | **1.942** | **42 / 6.2 at 8 t** | **15.4** | EPYC 9R45, 10^9 u64 keys, one query at a time (PHastR Table 3); PtrHash compact λ=3.9 on the same table: 2.143, 125 / 23.8, 29.3 |
| PHOBIC λ=4.5 IC-R | 2.11 | 254 / 40 at 8 t | 57 | i7-11700, 10^8 strings (PHOBIC Table 1) |
| Consensus-RecSplit | 1.444 | 30,245 / – | 199 | i7-11700, 10^8 strings, 1 t |

The survey's verdict: "PtrHash and PHast … are a good initial choice in practice".
External-memory construction at 10^9–10^12 keys is solved by partitioning by hash bits
and solving parts independently: PTHash-HEM (7.4·10^9 strings, 16 GB budget, HDD), PtrHash
sharding (5·10^10 keys on a 64 GB laptop), PHastR offline (10^12 keys, ≤ 0.46 B/key RSS).
Our design (spilling dedupe, parts of 2^15 solved independently) is the same pattern.

### 2.2 Parsing and decompression (#32)

| System | Rate | Conditions |
|---|---|---|
| csveee (Rust, portable SIMD) | 1.8–2.1 GB/s per thread record counting; 186 GB/s integer column, 40 GB/s all 16 columns converted | EPYC 9745 128 c, 1 TB DDR5; peak 181 GB/s unquoted at 198 threads |
| simdjson | 2.2 GB/s, one core | i7-6700; stage 1 is 0.5–1 cycle/byte; numbers cost dozens of cycles each |
| GVEL integer edge-list parser | ~1 GB/s per physical core (agent's arithmetic: 1.9 B edges/s, ~30 GB/s) | 2× Xeon 6226R, 32 c / 64 t, page cache |
| DuckDB 1.4.4 | 0.6 GB/s one thread, ~8 GB/s at 256 threads | csveee's machine |
| Instant Loading | > 1.6 GB/s at 8 threads | i7-3770, 2013 |
| **Ours (#32)** | **0.72 GB/s per thread (~7 cycles/byte); 7.3–8.2 GB/s at 16 threads** | 7950X, page cache, no SIMD |
| rapidgzip | 1.86 GB/s at 16 cores without index (4.25 with); 0.15–0.17 one thread | 2× EPYC 7702, Silesia |
| pzstd | 6.78 GB/s at 16 cores only on multi-frame files; single frame 0.88 | same |

Exact chunk-start resolution for quoted CSV exists without speculation: ParPaRaw composes
per-chunk state-transition vectors by a parallel scan; csveee and Ge et al. speculate and
validate (74.6% / 95.6% of chunks right on the first / second guess in csveee; Ge: one
misprediction in 10^7 chunks), but csveee's false-positive re-parse is not implemented.

### 2.3 Building the CSR (#35, promise 1)

| System | Input → output | Rate | Conditions |
|---|---|---|---|
| **Ours (lab, harness)** | text → sorted, deduplicated, validated symmetric CSR, persisted, deterministic, under a budget | orkut 150 M edges/s (≈ 300 M entries/s, ~19 M entries/s per core); Friendster 62 M sustained, 92–102 rested | 7950X 16 c / 32 t, dual-channel DDR5 |
| PIGO | Friendster text → CSR in RAM, unsorted lists, no dedup, nondeterministic order | 3.5 s for "3.6 B edges" ≈ 1 G entries/s, ~28 M per core (if CSR included; paper ambiguous) | 2× E5-2695 v4, 36 c / 72 t, warm cache |
| GVEL | MTX → CSR in RAM | 2.6× PIGO on edge lists, 1.8× on CSR | 32 c / 64 t |
| DovetailSort / semisort | CSR transpose, sort only | Twitter 1.47 B in 0.714–0.888 s, ~17–21 M entries/s per core | 4× Xeon 6252, 96 c |
| Ringo | table → graph | lj 13.0 M edges/s, Twitter 18.0 M | 4× E7-4870, 80 threads, 1 TB |
| ParaGrapher | loads an already compressed CSR | 260 M edges/s from SSD | W-2295 18 c, PCIe 4 NVMe |

Sorting scales about **8× on a 16-core single-socket desktop** (IPS4o: 8.37× on a Ryzen
3950X, 2^30 u64; 40.9× on a 64-core EPYC) because the permutation phase is memory-bound.
After relabelling, ids are dense and n ≪ m: the counting-sort regime (DovetailSort says so
explicitly), which is what our bucket design already is. No paper opened states a
bytes-written bound for out-of-core CSR construction.

### 2.4 Formats and containers (D7, D9)

No standard combines, in one file, arbitrary non-tabular sections, crash-safe append, a
hash per section, 4 KiB alignment and deterministic bytes. GraphAr and Lance are
multi-file datasets; Arrow IPC, Parquet and Vortex need a footer rewrite to append; F3 and
AnyBlox are research prototypes built around embedded Wasm decoders (AnyBlox files are
limited to 4 GB; F3's Wasm decoding costs 15–46% per column and 9.6–25% end to end). The
lessons that transfer: dictionary-encode, keep encodings light and **leave block
compression off by default** (up to 4.2× scan overhead, Zeng et al., i3.2xlarge NVMe);
keep statistics in a central random-access table (Zeng; Lance); align pages (Lance:
unaligned pages cost 2–3 sectors per I/O); fixed-width Arrow buffers random-access in one
I/O (Lance). GraphAr: still incubating (0.13.0, 2025-08-29); its Rust crate is cxx
bindings to the C++ library; its authors say it is not the store for iterative whole-graph
analytics.

### 2.5 Engines and what they give users

| Engine | Strength users get | Weakness for our users |
|---|---|---|
| Kùzu (archived 2025-10) → LadybugDB (MIT) | Cypher; factorized m-n joins; ASP-Join with semijoin filters so scans read only needed blocks; WCO joins (triangle on lj 0.04–59 s by selectivity, 8 threads, 2× E5-2670); edge properties in CSR parallel to adjacency, double-indexed | algorithms in an extension that failed to load in our lab; shortest path 2.55 s on lj |
| DuckDB + DuckPGQ | SQL/PGQ; on-the-fly CSR from tables; **bulk many-to-many path finding with bit-parallel MS-BFS** (16K pairs; 6× on 16 threads, 48 vCPU Xeon 8375C); "perfect hash join" when keys are dense | did not load on DuckDB 2.0.1 in our lab; research project; USING KEY shown only on graphs of ≤ 1,618 nodes |
| Neo4j GDS | **memory estimation before every run, automatic refusal** (`.estimate`, `bytesMin/Max`, `sudo: true` to override; heap only, conservative); graph catalog; filtered projection `gds.graph.filter` with Cypher predicates, `degree()`, `$parameters` | projections are copies in the JVM heap; 51 s load of lj in our lab |
| graph-tool | `GraphView(g, vfilt=, efilt=)`: mask views that share data, stackable, "no overhead when not filtered" | GPL-style licence terms not stated on the page; no PyPI wheels |
| NetworKit | parallel kernels; **network profiles** — one call gives a report of distributions, centrality correlations, component / community / k-shell sizes ("hundreds of millions of edges in minutes", 2× E5-2680) | in-RAM only |
| igraph | breadth of algorithms and formats | single-threaded kernels |
| Raphtory (GPL-3.0) | temporal views: windows, **rolling / expanding windows**, layers (edge types), lazy views, temporal motifs; Rust + Python | on-disk only in the paid edition |
| Graphsurge | **view collections** (many windows) computed differentially, adaptive split vs from scratch: up to 1.9× over the better baseline, ordering up to 10.1× | research, Timely/Differential Dataflow |
| Kaskade | cost-based choice of materialized graph views under a space budget, up to 50× (Neo4j 3.2, 28 cores) | heterogeneous-schema queries |
| Grafeo (Apache-2.0, Rust, 0.6.x) | GQL, Cypher, Gremlin, SPARQL, SQL/PGQ; WASM; spilling | self-reported benchmarks only |
| NetworkX dispatch (3.5–3.7) | `backend_priority` with `algos`, **`generators`** (functions that return graphs — including `read_edgelist`, `parse_edgelist`, `from_pandas_edgelist`, `from_scipy_sparse_array`, all `returns_graph=True`) and **`classes`** (`nx.Graph(backend=…)`, 3.6); conversion cache | nx-cugraph is **deprecated: RAPIDS 26.12 is its last release** |

### 2.6 Budgeted and anytime queries

Fan, Wang, Wu (SIGMOD 2014) answer a query by reading at most α|G|: for pattern queries
with locality they often reach 100% accuracy at α = 0.0015% (Youtube, Yahoo, i7-3520M
8 GB); for reachability they prove that no algorithm decides it with 100% accuracy for
α < 1 without an index, and use a landmark index that guarantees no false positives. Our
`limit`/`timeout` with `truncated` is the honest form of this (a cut search says "may
exist", which § 3.7 of the API sketch already does). SQL/PGQ's `ANY SHORTEST` is
nondeterministic by definition (DuckPGQ § 1); our rule for which shortest path is returned
is a real difference.

---

## 3. Decision by decision

| Decision / issue | Best known work | Us | Evidence |
|---|---|---|---|
| **D8 / #34** MPHF | PHastR: 1.94 bits/key, 15.4 ns query, 42 ns/key build per thread (EPYC, 10^9 u64) | **Behind on space and build per thread; level where it matters.** Our 2.61 bits/key = PtrHash's default row exactly (2.29 + 0.33) | The MPHF is ~1.3% of a 20-byte-key dictionary (~200 bits/key with slot→id and keys; A's arithmetic); pilot arrays at 500 M keys (143 vs ~118 MB) are both larger than the L3. `ph` and `sux` use `unsafe` and heavy dependencies; `ph`'s output depends on the thread count; `sux` is thread-independent only with `sweep_chunks` fixed. |
| D8 dictionary design (MPHF + slot→id + key store) | Same pattern in PTHash-HEM / PtrHash sharding / PHastR offline | **Level.** A plain hash table of 500 M × 24 B is ~13.7 GB (over the 4 GiB target); sort-and-rank means an external sort of 43–58 GB (A's arithmetic) | — |
| D8 mapping loop (3.6 B endpoints) | PtrHash app. A: staged batches with no dependency between keys, streaming at 8 ns/key, DRAM-bound | **Probably behind** (unmeasured): `id_of_member` does two dependent misses (pilot, then slot→id) and reads each `u32` through a `dyn ByteSource` call (`ids/dict.rs:757`, `:1257`) | Estimate: 3.6 B × 2 misses ≈ 6–8 s of DRAM traffic against ~27 s at the unverified 132 M/s. |
| D8 solver details | PtrHash: eviction scan starts at a random pilot, limit 10·S | **Slightly behind**: our scan starts at pilot 0, limit `20 * n + 10_000` (`ids/mphf.rs:259`) | Starting point can be a hash of (seed, bucket, eviction count), still deterministic. |
| **D7 / #28** `.eg` container | No standard with sections + A/B commit + per-section hash + alignment + deterministic bytes | **Ahead for our needs; keep.** | GraphAr/Lance multi-file and immutable; Arrow IPC/Parquet/Vortex need footer rewrites; Vortex has no checksums; F3 has checksums but is a prototype. Two reads to open (header, TOC) like Vortex; PMTiles does one (16 KiB). |
| D7 codec rules | BtrBlocks chooses encodings by `rand()` sampling | **Gap**: nothing in D7 forbids a randomly sampled encoder | Add "a codec is a pure function of the section's input". |
| **D9 / #37** property columns | Kùzu stores edge properties in CSR parallel to the adjacency, double-indexed; Arrow IPC is the zero-copy interchange | **Level on layout; open on framing.** Arrow IPC framing inside sections, with bodies at 4 KiB offsets and no IPC compression, makes `pyarrow` read them zero-copy without Everygraph (#37's acceptance) | Lance/Zeng: fixed-width buffers need one I/O; Arrow buffers must be ≥ 8-byte aligned (64 recommended), ≤ 2^31−1 elements per array, Utf8 offsets 32-bit. **D9's fixed "u32 edge id" caps a graph at 2^32−1 edges** (m2-10 already says u32/u64; DESIGN does not). |
| **D10 / #93** views | graph-tool: mask views sharing data; GDS: per-run filter vs materialized `gds.graph.filter`; Kaskade: cost-based materialization under a space budget; Graphsurge: adaptive differential vs from scratch from runtime statistics; Kùzu: semijoin filters so a scan reads only needed blocks | **Level on the idea, behind on the mask we measured.** The lab's mask was an *entry* bitset that still visits every entry; a *node* mask can skip whole lists through the per-node Elias–Fano offsets (D4), and block zone maps can skip whole ~64K-edge blocks for time/weight predicates | Planner-chosen representation is not novel (Kaskade, Graphsurge, GDS); "identical result either way" is ours. |
| **#94** time windows | Raphtory `rolling`/`expanding`; Graphsurge view collections | **Behind** on many windows: one `window()` at a time | Graphsurge: 1.5–13.9× for differential over from-scratch on similar views (SO dataset, 2× E5-2670). |
| **D19 / #81, #83** PyO3 + Arrow | Kùzu, DuckDB, LadybugDB, Grafeo all ship Python with Arrow | **Level.** | — |
| **D20 / #86** NetworkX backend after 0.1 | NetworkX ≥ 3.5/3.6: dispatchable readers and constructors return backend graphs; nx-cugraph ends at RAPIDS 26.12 | **The premise has moved.** ADR 0020 rests on "on an existing `nx.Graph`, conversion dominates" and says it would change its mind if NetworkX gained a way to hand a graph to a backend without conversion. For code that loads through `nx.read_edgelist` / `nx.from_pandas_edgelist`, with `backend_priority.generators` set, the graph is built by the backend: no conversion. The only thriving backend is leaving | NetworkX 3.7 docs; `networkx/readwrite/edgelist.py` and `convert_matrix.py` (`@nx._dispatchable(graphs=None, returns_graph=True)`); nx-cugraph README. Not measured by us. |
| **D21 / #90, #91, #110** traversal API, no query language | DuckPGQ: bulk many-to-many paths with MS-BFS; Kùzu/Ladybug/Grafeo: Cypher/GQL | **Level for v0.1**; one gap: no public name for a batch of pairs (#91 scope has "Batch pairs API"; api-sketch § 7 "no name here") | MS-BFS: over an order of magnitude over single BFSs when running many (60-core Xeon E7-4870 v2). The kernel is already planned (#49). |
| **Promise 1 / ADR 0026 / #35** | PIGO two-pass (count, prefix sum, scatter in place); GVEL; counting sort for dense keys | **Level per core, ahead in scope** (nobody publishes deterministic, budgeted, persisted text → CSR), **behind 3–5× in absolute rate** against 2-socket servers doing less work, and **behind 3–5× in bytes written** on Friendster | 32 GB budget writes 29.6 GB of temp (one symmetrized spill) + CSR; an in-memory two-parse build would write only the output (~10 GB at 23.2 bits/entry). |
| #35 sampled-quantile partition | Exact boundaries from the degree histogram of pass 1 | **Unneeded** if degrees are counted first | Boundaries never change the canonical output. |
| #35 thread scaling (3.4–4.9× 1→32) | IPS4o 8.37× on a 16-core 3950X | **Behind ~2×** of the realistic ceiling, not of 32× | B's arithmetic: orkut moves ~8–10 GB in 0.78 s (~11 GB/s), far below bandwidth: serial phases or page faults, not memory. |
| **#32** text reader | csveee 1.8–2.1 GB/s/thread (Rust, portable SIMD); simdjson | **Behind 2–3× per thread** on the number path; level with DuckDB | End to end only 10–25% (parse ≈ 4 s of Friendster's 29 s, ≈ 0.2 s of orkut's 0.78 s; B's estimate). |
| #32 gzip | rapidgzip 1.86 GB/s at 16 cores; pure-Rust `rapidgzip-core` exists (pre-1.0, unverified) | **Behind 3–6×** | Exact decompression: no determinism risk. |
| **#33** Parquet reader | — | — | Nothing found against D9/#33; reading Parquet through arrow-rs stays outside the core. |
| **m6-05** GraphAr | 0.13.0, incubating; Rust crate = C++ bindings | Plan is right: interchange later | Writing it needs only YAML + parquet-rs. |

---

## 4. Challenges, ranked

Each: change · expected gain (source) · cost and risk against determinism, budget, wasm32,
no C/C++ · cheapest experiment · issue.

**C1. Build without a spill when the targets fit; re-read instead of spilling when they
do not.** Pass 1 counts degrees by original id and builds the rank directory; pass 2
scatters into an O(m) targets array (Friendster 3.6 B entries × 4 B = 14.4 GB, which fits
a 32 GB budget); then per-list sort + dedup (restores the canonical order, so the scatter
may be nondeterministic) and the streaming encoder. Under 8 GB, re-read the input once per
source range instead of spilling (or one text → compact binary pass when the input is gzip
or string-keyed). *Gain:* bytes written fall from 45.1 / 55.4 GB to the output (~10 GB);
estimate 15–22 s at 32 GB, i.e. ADR 0026's "≤ 20 s would let the promise drop its second
sentence" threshold (B's estimate from our own phase times; PIGO's two-pass structure).
*Cost:* a second build path, or a planner rule choosing passes; gzip and MPHF lookups paid
per pass. *Risk:* none for determinism (sorted lists); the budget rule becomes "targets
array fits → in memory". *Experiment:* interleaved A/B in the lab crate, Friendster,
harness warm-up protocol, 32 GB and 8 GB: current pipeline vs two-parse, uncompressed
output for both; wall time, cgroup `io.write_bytes`, PSI io stall. *Issue:* #35 (amend the
"Improvements" list); ADR 0026's consequences.

**C2. Stage the id-mapping loop.** Hash a batch → part and bucket → gather pilots → slots
→ gather slot→id, with no dependency between keys, on slices rather than per-element
`dyn ByteSource` calls. *Gain:* up to ~3–4× on mapping 3.6 B endpoints (estimate from two
DRAM misses per endpoint vs the unverified 132 M/s; PtrHash streams at 8 ns/key, i7,
10^9 keys). *Cost:* small, safe Rust, wasm-safe; explicit prefetch only if callable without
`unsafe`. *Risk:* none (same ids). *Experiment:* per-key loop vs batches of 64/256/1024,
interleaved in one process, 500 M 20-byte keys and a 10^9-endpoint stream, 1 and 32
threads; endpoints/s and `perf stat` L3 misses. Also: hashed eviction start and a 10·S
limit, evictions per part over 1,000 parts of 2^15. *Issue:* #34 follow-up or #25.

**C3. Re-open the timing of D20.** Set `backend_priority.generators` (and `classes`) for
an `nx-everygraph` backend so that `nx.read_edgelist(…)` / `nx.from_pandas_edgelist(df)`
build a native graph and every later call runs without conversion. *Gain:* the measured
"kernel only" ratios (PageRank 15.6–70×, components 24–53× on lj, 3 threads, shared
machine) become reachable on unchanged NetworkX scripts that load from files or
DataFrames — the case ADR 0020 said would change its mind; the channel loses its only
large competitor (nx-cugraph's last release is 26.12). *Cost:* the backend is still 2–3
engineer-months for ~60 functions (lab estimate); exact NetworkX semantics remain the hard
part. *Risk:* adoption story and caveats; no core risk. *Experiment:* a 1-day prototype on
the lab backend: `nx.read_edgelist` of lj with and without `backend_priority.generators`,
then `nx.pagerank`; time from file to result; NetworkX 3.6+. *Issue:* #86; ADR 0020
"What would change our mind" (record that it may have fired).

**C4. Arrow IPC framing for column sections, and a variable edge-id width.** Each column
section is a complete Arrow IPC file: schema, record batches whose bodies start at 4 KiB
offsets, no IPC compression, fixed batch row count (≤ 2^31−1 elements, < 2 GiB bodies),
dictionaries in the same section; the TOC or a side table records each values buffer's
absolute offset so kernels never parse FlatBuffers. Write the framing ourselves (a few
hundred lines) and pin it with golden bytes; LargeUtf8 or dictionary encoding for big
string pools; edge-id width u32 when m < 2^32, else u64. *Gain:* `pa.ipc.open_file` on the
slice of a memory-mapped `.eg` reads zero-copy without Everygraph — #37's acceptance —
and Polars reads the same bytes (C, from the Arrow spec and arrow-rs source). *Cost:* small.
*Risk:* byte determinism if arrow-rs's writer changed — hence our own writer. *Experiment:*
~1 hour, no graph: 2^27-entry f32 column in two batches plus a dictionary-encoded u8
column; `open_file`, buffer address inside the map and `% 4096 == 0`, round trip, identical
bytes at 1 and 32 threads. *Issue:* #37; DESIGN D9 wording ("u32 edge id").

**C5. Views that skip work: node masks over list offsets, block zone maps, then rolling
windows.** #93 should measure, beside the entry bitset the lab tried, a node mask that
skips non-selected lists through the per-node Elias–Fano offsets, and per-block min/max of
time and weight columns (zone maps aligned to the ~64K-edge adjacency blocks) so a window
or `value > x` skips whole blocks. After v0.1: `g.rolling(…)` / view collections. *Gain:*
masks whose cost follows the selection instead of m (Kùzu's sip argument; graph-tool's
"no overhead"); finer zone maps help low-selectivity filters (Zeng et al.); differential
view collections 1.5–13.9× (Graphsurge). *Cost:* zone maps are one optional section; node
masks reuse D4's index. *Risk:* results must stay identical to the materialized view
(D14) — same summation order needed. *Experiment:* add "node mask with list skipping" and
"time window with block zone maps" to #93's crossover grid (orkut, twitter-sym, 16
threads, 1–90%). *Issue:* #93, #94; zone maps also #37.

**C6. Parsing: SWAR integers and exact quote parity.** Convert 8 digits at a time with u64
SWAR (portable, native on wasm32), keep a no-quotes fast path, and resolve chunk starts in
quoted CSV exactly by composing per-chunk quote parity (or a two-state transition vector
for backslash dialects; ParPaRaw), re-parsing only mismatched chunks. *Gain:* parse
7.3–8.2 → ~15 GB/s at 16 threads (estimate); 10–25% end to end. *Cost:* moderate.
*Risk:* none if exact (do not use csveee/Ge speculation as the correctness mechanism).
*Experiment:* parse-only, scalar vs SWAR, orkut text in page cache, 1 and 16 threads,
interleaved; GB/s and cycles/byte. *Issue:* #32 follow-up.

**C7. Exact bucket boundaries and a per-phase scaling profile.** Take bucket boundaries
from the cumulative degree after pass 1 (balanced and deterministic), drop the
sampled-quantile item, and set the scaling expectation to ~8×, not 32×. *Gain:* 1.7–2.4×
on small in-RAM graphs if the missing scaling is serial phases (B's estimate). *Cost:* low.
*Experiment:* per-phase times and GB/s at 1, 2, 4, 8, 16, 32 threads on orkut, one
harness run. *Issue:* #35.

**C8. Faster compressed input.** Feature-gated parallel gzip outside the wasm core
(`rapidgzip-core`, pure Rust on zlib-rs, pre-1.0, unverified) and multi-frame zstd for
everything we write. *Gain:* Friendster `.gz` inflate from ~60–100 s to ~10–18 s
(estimate from 1.86 GB/s at 16 cores). *Risk:* dependency maturity; the claim of parity
with C++ rapidgzip is unverified. *Experiment:* decompress-only throughput, friendster
`.gz`, 16 threads, vs flate2, interleaved. *Issue:* #32 / #25.

**C9. API items to copy for v0.1.** (a) Node tables: `eg.read(…, nodes=table)` — every
engine compared (Kùzu, GDS, igraph, graph-tool) has node properties, #33 already reads
node tables, and api-sketch § 7 says the name needs a preview. (b) A public name for the
edge-type column (Raphtory layers, GDS relationship types, Kùzu rel tables); #32/#33 have
the reader option without a name. (c) A batch of path pairs (`g.paths(table)` or
similar) on the planned MS-BFS kernel (DuckPGQ). (d) A graph profile in one call
(NetworKit profiles: degree, component, core, community size distributions), mostly
assembling kernels v0.1 already has. (e) Bound parameters in filters
(`g.filter(edges="value > $x", params={"x": 10})`, as GDS), so users do not format numbers
into strings. *Cost:* (a)–(c) need previews (D25); (d) is a small issue; (e) is a grammar
addition. *Issue:* api-sketch § 7; #81, #91; new: "Graph profile in one call".

**C10. A rule for codecs.** "A codec is a pure function of the section's input; any
sampling is seeded by position." *Gain:* keeps D7's identical bytes when an adaptive codec
(BtrBlocks-, Vortex-style) is added. *Cost:* one sentence in `eg-container.md`. *Issue:*
#28.

**C11. Record PHastR, change nothing.** Note in ADR 0008 that PHastR (1.94 bits/key,
faster per thread) is the option to revisit if part solving exceeds ~20% of a dictionary
build's wall time; no work on MPHF space (λ, compact pilots, EF remap). *Experiment:*
phase timings inside one 500 M-key build, 32 threads. *Issue:* #34.

**C12. Interchange crates outside the core.** GraphAr export/import (YAML + parquet-rs,
no C++), WebGraph BV import (LAW datasets; the ratio baseline for D4). *Issue:* m6-05,
#15-style experiment for BV random access vs `eg-adjacency`.

---

## 5. What we have that the literature does not — checked

| Claim | Verdict |
|---|---|
| Text → CSR that is deterministic for any thread count and memory budget, persisted, validated | **Not found** in the loaders read (PIGO, GVEL, Ringo, ParaGrapher, GAP, GBBS); PIGO/GVEL scatter with atomics and keep lists unsorted. |
| Plan before running and refuse when it does not fit | **Not new**: GDS estimates every run and refuses when heap is clearly insufficient (`sudo` overrides). New is the hard budget with an out-of-core regime that returns the same answer. |
| Views held as mask or materialized, chosen by a planner | **Not new**: Kaskade (cost model, space budget), Graphsurge (adaptive from runtime statistics), GDS (per-run filter vs `gds.graph.filter`), graph-tool masks. New: the identical-result guarantee across both. |
| Deterministic MPHF build at any thread count | **Not unique**: PHastR is thread-independent with fixed chunks; PtrHash sharding aims at strategy-independent output. Unique in an ingest dictionary with spilling dedupe and a published stored form. |
| One file with content-hashed sections and crash-safe append | **Not found for graphs** (GraphAr multi-file, nkbg's checksum field unused, `.gt` none). The pieces exist elsewhere (F3 checksums, PMTiles single file, A/B headers in databases). |
| Budgeted traversals that say they were cut and keep a defined set of rows | Resource-bounded querying exists (Fan et al., accuracy-oriented, approximate); deterministic `limit` row sets were not found. |
| A deterministic choice among equal shortest paths | SQL/PGQ `ANY SHORTEST` is nondeterministic by definition; `ALL SHORTEST` is the deterministic alternative there. Ours is a documented rule — a real difference, small. |

---

## 6. What to ignore, and why

- **Consensus-RecSplit, ShockHash, RecSplit, SicHash, BBHash, FMPH, PHOBIC, PTHash-HEM** — smaller or older, all 2–10× slower to query; the MPHF is ~1% of the dictionary.
- **`ph` and `sux` as dependencies** — `unsafe`, heavy dependency trees, thread-dependent output (`ph`).
- **Non-minimal k-perfect bins** — one miss per lookup, but 3.3–4.4 GB of bins at 500 M keys (A's estimate); research grade.
- **csveee / Ge speculation as the correctness mechanism** — an oracle plus an unimplemented re-parse cannot give byte-identical output.
- **GPU parsing (ParPaRaw on GPU), GPU RecSplit** — no GPU in the core.
- **ParaGrapher / CompBin** — loads an already built compressed CSR from WebGraph (Java back end); our decode path is already cheap.
- **Lance, Vortex, Nimble, F3, AnyBlox as containers** — multi-file, 0.x, C++/unstable, prototype, or a 4 GB Wasm ceiling; take their lessons only.
- **Parquet sections inside `.eg`** — encoded pages, no zero-copy; Parquet stays an import/export format.
- **FastLanes for adjacency** — 1024-value vectors do not fit short variable-length lists; maybe later for time columns.
- **HDF5 / Zarr** — array formats little supported by the tools our users join with (Zeng et al.).
- **Bao / BLAKE3 verified sub-range reads, single-request open (PMTiles)** — nice later, not 0.1.
- **Factorized processing and WCO joins (Kùzu, Mhedhbi–Salihoglu)** — only matter with a pattern-matching language, which is #110 after 0.1.
- **SQL/PGQ, USING KEY recursive CTEs** — DuckDB's own demonstration uses graphs of at most 1,618 nodes; not our competitor at 10^9 edges.
- **GraphScope Flex / GRIN** — distributed stack with a C storage interface; our `ByteSource` + edge-map framework already decouples storage.
- **Raphtory's GraphQL server, Kaskade's schema-mined views** — server and heterogeneous-schema features outside v0.1.
- **Ringo-style table joins to build graphs (bipartite projections)** — users can do the join in DuckDB/Polars and hand the edge table over; a projection operator risks m² blow-ups.
