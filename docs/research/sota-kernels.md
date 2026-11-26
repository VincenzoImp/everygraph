# State of the art: shared-memory parallel graph kernels and deterministic parallelism

Survey of 2026-11-25 for D12 (edge-map framework, #44), D13 (threads per kernel), D14
(deterministic by default) and the M3 kernel issues (#47–#52, #54) plus #18 (deterministic
Leiden) and #22 (memory bandwidth). Builds on `docs/research/memory-bounded-algorithms.md`
(out-of-core variants are not repeated here). Nothing was built, run or measured; every
number below is a paper's, given with its machine, graph and thread count, and is **not
comparable to ours as-is**: almost every reference machine is a 2- or 4-socket server with
2–4× the memory channels of the lab's Ryzen 9 7950X (16 cores / 32 threads, dual-channel
DDR5, 2 × 32 MB L3). "Inference" marks my reasoning, not a paper's result.

## 1. Scope and reading log

Read level: **full** = method and evaluation read; **full (delegated)** = read in full by a
reader agent briefed with our design, its key numbers spot-checked by me against the text
(grep of the `.txt`); **abstract/results**; **not accessible**. PDFs in
`/home/tux/graphlab/papers/kernels/` (text in `txt/`).

| # | Work | Venue, year | Link (verified, downloaded) | Read level | PDF |
|---|---|---|---|---|---|
| 1 | Ligra (Shun, Blelloch) | PPoPP'13 | https://people.csail.mit.edu/jshun/ligra.pdf | full (delegated) | 2013-shun-ligra.pdf |
| 2 | Ligra+ compression (Shun, Dhulipala, Blelloch) | DCC'15 | https://people.csail.mit.edu/jshun/ligra+.pdf | full (delegated) | 2015-shun-ligraplus.pdf |
| 3 | Julienne bucketing (Dhulipala, Blelloch, Shun) | SPAA'17 | https://people.csail.mit.edu/jshun/bucketing.pdf | full (delegated) | 2017-dhulipala-julienne.pdf |
| 4 | GBBS, "Theoretically efficient … fast and scalable" | SPAA'18; arXiv v4 = TOPC'21 | https://arxiv.org/abs/1805.05208 | full (delegated); determinism grep by me | 2018-dhulipala-gbbs.pdf |
| 5 | PASGAL (Dong, Gu, Sun, Wang) | SPAA'24 BA | https://arxiv.org/abs/2404.17101 | full (delegated) | 2024-dong-pasgal.pdf |
| 6 | ParlayLib (Blelloch, Anderson, Dhulipala) | SPAA'20 BA | https://www.cs.cmu.edu/~guyb/papers/3350755.3400254.pdf | full (delegated) | 2020-blelloch-parlaylib.pdf |
| 7 | Direction-optimizing BFS (Beamer, Asanović, Patterson) | SC'12 | http://scottbeamer.net/pubs/beamer-sc2012.pdf | full (delegated); α/β checked | 2012-beamer-dobfs.pdf |
| 8 | GAP Benchmark Suite (Beamer et al.) | arXiv 2015/17 | https://arxiv.org/abs/1508.03619 | full (delegated) | 2015-beamer-gap.pdf |
| 9 | Frameworks on GAP (Azad et al.) | IISWC'20 | https://www.cs.utexas.edu/~roshan/GraphAnalyticsFrameworksStudy.pdf (via web.archive.org) | full (delegated) | 2020-azad-gap-frameworks.pdf |
| 10 | Galois (Nguyen, Lenharth, Pingali) | SOSP'13 | https://sigops.org/s/conferences/sosp/2013/papers/p456-nguyen.pdf | full (delegated) | 2013-nguyen-galois-sosp.pdf |
| 11 | GraphIt (Zhang et al.) | OOPSLA'18 | https://arxiv.org/abs/1805.00923 | full (delegated) | 2018-zhang-graphit.pdf |
| 12 | Ordered GraphIt, bucket fusion (Zhang et al.) | CGO'20 | https://arxiv.org/abs/1911.07260 | full (delegated); orkut row checked | 2020-zhang-graphit-ordered.pdf |
| 13 | LAGraph (Szárnyas et al.) | GrAPL'21 | https://arxiv.org/abs/2104.01661 | full (delegated) | 2021-szarnyas-lagraph.pdf |
| 14 | Push-pull in GraphBLAS (Yang, Buluç, Owens) | ICPP'18 | https://arxiv.org/abs/1804.03327 | full (delegated) | 2018-yang-graphblas-pushpull.pdf |
| 15 | SuiteSparse:GraphBLAS, Algorithm 1037 (Davis) | TOMS'23 | https://doi.org/10.1145/3577195 | **not accessible** (ACM 403); performance known via 9 and 13 | – |
| 16 | Internally deterministic algorithms (Blelloch, Fineman, Gibbons, Shun) | PPoPP'12 | https://www.cs.cmu.edu/~guyb/papers/BFGS12.pdf | full (delegated) | 2012-blelloch-internally-deterministic.pdf |
| 17 | Greedy MIS/matching parallel on average | SPAA'12 | https://arxiv.org/abs/1202.3205 | full (delegated) | 2012-blelloch-greedy-mis.pdf |
| 18 | Priority updates (Shun et al.) | SPAA'13 | https://people.csail.mit.edu/jshun/contention.pdf | full (delegated) | 2013-shun-priority-update.pdf |
| 19 | Phase-concurrent hash tables (Shun, Blelloch) | SPAA'14 | https://people.csail.mit.edu/jshun/hash.pdf | full (delegated) | 2014-shun-phase-concurrent-hash.pdf |
| 20 | Deterministic Galois (Nguyen, Lenharth, Pingali) | ASPLOS'14 | https://iss.oden.utexas.edu/Publications/Papers/nguyen14.pdf | full (delegated); 4.2× checked | 2014-nguyen-deterministic-galois.pdf |
| 21 | Deterministic Mt-KaHyPar, SDet (Gottesbüren, Hamann) | Euro-Par'22 | https://arxiv.org/abs/2112.12704 | **full** (me: §3, §4.4–4.5) + delegated | 2021-gottesburen-det-mtkahypar.pdf |
| 22 | DetJet / DetFlows (Krause, Gottesbüren, Maas) | arXiv 2025 | https://arxiv.org/abs/2504.12013 | full (delegated); 15/16/24% checked | 2025-gottesburen-detjet.pdf |
| 23 | Phase-parallel iterative algorithms (Shen, Wan, Gu, Sun) | SPAA'22 | https://arxiv.org/abs/2205.13077 | full (delegated) | 2022-shen-phase-parallel.pdf |
| 24 | Internally deterministic union-find (Fedorov et al.) | SPAA'23 | https://arxiv.org/abs/2304.09331 | full (delegated); results are figures only | 2023-fedorov-det-union-find.pdf |
| 25 | Parallel reproducible summation (Demmel, Nguyen) | IEEE TC'15 | https://aspire.eecs.berkeley.edu/wp/wp-content/uploads/2014/07/Parallel-Reproducible-Summation.pdf (via web.archive.org) | full (delegated) | 2015-demmel-parallel-reproducible-sum.pdf |
| 26 | Counter-based RNGs, Philox (Salmon et al.) | SC'11 | https://www.thesalmons.org/john/random123/papers/random123sc11.pdf | abstract/results | 2011-salmon-philox.pdf |
| 27 | ConnectIt (Dhulipala, Hong, Shun) | VLDB'21 | https://arxiv.org/abs/2008.03909 | full (delegated); Table checked | 2020-dhulipala-connectit.pdf |
| 28 | Afforest (Sutton, Ben-Nun, Barak) | IPDPS'18 | https://mosix.cs.huji.ac.il/pub/Parallel_Graph_Connectivity.pdf (via web.archive.org) | full (delegated); results are figures only | 2018-sutton-afforest.pdf |
| 29 | ρ-stepping, Δ*-stepping (Dong, Gu, Sun, Zhang) | SPAA'21 | https://arxiv.org/abs/2105.06145 | full (delegated) | 2021-dong-rho-stepping.pdf |
| 30 | Wasp (D'Antonio, Mai, Tsigas, Vandierendonck) | SC'25 | https://doi.org/10.1145/3712285.3759872 | **not accessible** (ACM 403); short version read in full: https://2025.debs.org/accepted-contributions/Toward_Efficient_Asynchronous_Shortest_Path.pdf | 2025-dantonio-wasp-debs.pdf |
| 31 | AdaMW (Wasp/MultiQueue selector) | FCPC'25 | https://pureadmin.qub.ac.uk/ws/portalfiles/portal/639672046/der.pdf | full (delegated) | 2025-dantonio-adamw.pdf |
| 32 | "Mind the Gap": natural vs synthetic SSSP weights | arXiv 2026 | https://arxiv.org/abs/2607.26821 | full (delegated) | 2026-mind-the-gap-sssp.pdf |
| 33 | Parallel k-core: theory and practice (Liu, Dong, Gu, Sun) | SIGMOD'25 | https://arxiv.org/abs/2502.08042 | full (delegated); table checked | 2025-liu-kcore.pdf |
| 34 | k-core with batched updates, async reads (Liu, Shun et al.) | arXiv 2024 | https://arxiv.org/abs/2401.08015 | abstract/results | 2024-liu-kcore-batched.pdf |
| 35 | PKT, describes PKC (Kabir, Madduri) | arXiv 2017 | https://arxiv.org/abs/1707.02000 | abstract/results | 2017-kabir-pkt.pdf |
| 36 | PKC itself (Kabir, Madduri, IPDPSW'17); ParK (Dasari et al., BigData'14) | 2017, 2014 | – | **not accessible**; known via 33, 35, 4 | – |
| 37 | Multicore triangle computations (Shun, Tangwongsan) | ICDE'15 | https://jshun.csail.mit.edu/triangle.pdf | full (delegated) | 2015-shun-triangles.pdf |
| 38 | LOTUS triangle counting (Koohi Esfahani et al.) | PPoPP'22 | QUB authors' copy, blogs.qub.ac.uk/DIPSA/…/LOTUS_TC_Authors_Copy.pdf | full (delegated); Friendster row checked | 2022-koohi-lotus.pdf |
| 39 | Parallel SCC via faster reachability (Wang, Dong, Gu, Sun) | SIGMOD'23 | https://arxiv.org/abs/2303.04934 | full (delegated) | 2023-wang-scc.pdf |
| 40 | FAST-BCC (Dong, Wang, Gu, Sun) | PPoPP'23 | https://arxiv.org/abs/2301.01356 | abstract/results | 2023-dong-biconnectivity.pdf |
| 41 | Propagation blocking (Beamer, Asanović, Patterson) | IPDPS'17 | http://scottbeamer.net/pubs/beamer-ipdps2017.pdf | full (delegated); friends row checked | 2017-beamer-propagation-blocking.pdf |
| 42 | Cagra, CSR segmenting (Zhang et al.) | BigData'17 | https://arxiv.org/abs/1608.01362 | full (delegated) | 2016-zhang-cagra.pdf |
| 43 | PCPM partition-centric PageRank (Lakhotia et al.) | arXiv'17, ATC'18 | https://arxiv.org/abs/1709.07122 | full (delegated) | 2017-lakhotia-pcpm.pdf, 2018-lakhotia-pcpm-atc.pdf |
| 44 | GPOP (Lakhotia et al.) | arXiv 2018 | https://arxiv.org/abs/1806.08092 | abstract only | 2018-lakhotia-gpop.pdf |
| 45 | DBG lightweight reordering (Faldu, Diamond, Grot) | IISWC'19 | https://arxiv.org/abs/2001.08448 | full (delegated) | 2019-faldu-dbg.pdf |
| 46 | MS-BFS "The more the merrier" (Then et al.) | VLDB'15 | https://www.vldb.org/pvldb/vol8/p449-then.pdf | full (delegated) | 2014-then-msbfs.pdf |
| 47 | Rusty-PBBS (Abdi, Zhang, Jeffrey) | SPAA'23 BA | https://www.eecg.utoronto.ca/~mcj/papers/2023.rusty-pbbs.spaa.pdf | **full** (me; no numbers in it) | 2023-rusty-pbbs.pdf |
| 48 | Graph analytics on Optane, Galois vs GAP/GBBS/GraphIt (Gill et al.) | VLDB'20 | https://arxiv.org/abs/1904.07162 | abstract/results (me) | 2020-gill-optane-galois.pdf |
| – | Δ-stepping (Meyer, Sanders 2003), MultiQueues (Rihani et al. 2015), Gorder, Rabbit order | – | – | not opened; covered through 8, 12, 29, 32, 45 | – |

## 2. The state of the art, as a map

**Frameworks.** Two families. (a) *Bulk-synchronous frontier frameworks*: Ligra → Ligra+
(compression) → Julienne (bucketing) → GBBS (20 problems, theoretically efficient,
Hyperlink2012 in 1 TB) → PASGAL (large-diameter graphs: VGC, hash bags). GraphIt makes
the same operators a scheduling space (push/pull/hybrid, bitvector frontiers, CSR
segmenting, NUMA) and adds ordered algorithms with bucket fusion. (b) *Asynchronous,
data-driven*: Galois (OBIM soft-priority worklists); on Optane-class large graphs it is on
average 1.6× faster than GBBS and 1.9× than GAP (48, abstract) — because sssp and cc run
asynchronously, which is nondeterministic in schedule. GraphBLAS/LAGraph is the
linear-algebra family; on the GAP graphs SuiteSparse:GraphBLAS is 1.1× (PageRank) to
~220× (SSSP Road, 46.79 vs 0.21 s) slower than the GAP reference, CC 3.4–20×, and faster
only for BC on 3 of 5 graphs (13; 1 × 20-core E5-2698 v4, 40 threads).

**Reference timings that define "best known" for our kernels** (paper machines; none ours):

| Kernel | Best known (algorithm, code) | Number (graph, machine, threads) | Typical implementation | Spread |
|---|---|---|---|---|
| PageRank (power-law, state ≫ LLC) | pull + cache blocking: propagation blocking (41), CSR segmenting (42, GraphIt), PCPM (43) | friends (124.8 M ids, 3.61 B edges): GAP pull 3.72 → DPB 2.12 s/iter, 1.75× (2 × 8-core Ivy Bridge, 16 threads, 2 × 25 MB LLC); GraphIt Friendster 20 it. 32.6 s = 1.63 s/iter (2 × 12-core Haswell, 48 HT) vs Ligra 10.05 s/iter | GAP-style Jacobi pull, "no longer competitive" (9) | 1.5–3× on large power-law graphs; none on graphs whose state fits LLC or with good layout |
| PageRank (high diameter) | Gauss–Seidel (in place) | IISWC'20 Road: 3.3–3.7× over Jacobi (2 × 16-core Xeon 8153, 32 threads) | Jacobi | order-dependent (see §3, D14) |
| WCC | ConnectIt: sampling (k-out hybrid / BFS / LDD) + UF-Rem-CAS, SplitAtomicOne | orkut 8.5 ms, Friendster 0.183 s, Twitter 0.092 s (4 × 18-core E7-8867 v4, 144 HT) | GAP Afforest: orkut 73 ms, Friendster 0.306 s, same machine | 1.3–8.6× over Afforest (avg 3.9×) |
| SCC | BGSS + hash bag + VGC (39) | LJ 0.038 s, Twitter 0.226 s (4 × 24-core Gold 6252, 192 HT); 4–19× over GBBS on lattices/k-NN | Multistep (FW-BW + coloring), iSpan, GBBS | baselines slower than sequential Tarjan in 24 of 36 large-diameter tests (39) |
| BFS (low diameter) | direction-optimizing (7), GBBS | orkut 12 ms (GBBS, 144 HT); 7 ms (GBBS, 192 HT, from 5) | GAP DO-BFS | within 2× of each other on power-law graphs |
| BFS (high diameter) | VGC multi-frontier (PASGAL), asynchronous (Galois), push-only (GraphIt) | road-Africa 0.05 s PASGAL vs 0.7 s GBBS vs 0.45 s sequential (192 HT) | level-synchronous with a barrier per level | up to 25× (IISWC'20 Road BFS spread) |
| SSSP | ρ-/Δ*-stepping (29), Wasp (30), MultiQueue for robustness (32) | Friendster 2.02 s ρ vs 2.95 s GAP (192 HT); Wasp 1.52 vs GAP 1.75 s (EPYC 7713) | GAP Δ-stepping (bucket fusion) | 1.3–2.6× on power-law; Δ mis-tuning costs up to 4.9× (32) |
| k-core | Liu et al. (33): active set + sampled degrees + VGC + hierarchical buckets | Friendster 3.68 s vs Julienne 6.18, PKC 33.1, ParK 416 s; Twitter 2.72 vs PKC 75.6 s (192 HT) | PKC / ParK / Julienne | 1.5–28× over PKC on social graphs; PKC competitive only on roads |
| Triangles | degree-ordered forward + merge (37, GBBS, GAP); LOTUS hub split (38) on skewed graphs | Friendster: LOTUS 56.7, GAP 70.5, GBBS 76.4 s (32-core SkyLakeX); Twitter LOTUS 10.0 vs GAP 32.1 s | GAP heuristic relabel + merge | LOTUS 3.2× on Twitter, 1.15–1.24× on Friendster |
| Multi-source BFS | MS-BFS bit-parallel (46) | all-vertices closeness 12–30× over one DO-BFS per source (4 × 15-core E7-4870 v2, 60 cores) | one BFS per source | – |

**Determinism, as the literature prices it.** Per-kernel determinism is cheap: priority
updates / writeMin cost ≈ 10% on low-contention BFS and win under contention
(18: 0.307 vs 0.28 s 3D grid; 0.08 vs 0.954 s 4-comb, 40 cores); deterministic hash tables
7–23% (19); deterministic BFS 0.314 vs 0.28 s (16, 32 cores / 64 HT); min-root union-find
gives the minimum id label for free (27, 28). Deterministic **local moving** done well is
also cheap: Mt-KaHyPar's deterministic coarsening is 16–24% slower than its nondeterministic
coarsening, DetJet 15% (22, EPYC 96 cores at 64 threads); SDet self-speedup 12.8× at 16
threads geomean (21, EPYC 7742). What is expensive is *generic* deterministic scheduling:
Deterministic Galois is a median 4.2× slower than nondeterministic Galois (20, 40 cores),
attributed to lost locality; order-free floating-point summation costs 3–7× (25).

## 3. Decision by decision

| Item | What the best known work does | Us | Evidence |
|---|---|---|---|
| **D12** one edge-map framework | GBBS: one framework (edgeMap / edgeMapData / edgeMapBlocked, vertexSubset sparse↔dense, bucketing, histogram, filter/pack, contraction) covers 20 problems; Ligra+ adds compression without changing the interface (byte-RLE +14% parallel, −3% sequential, 40 cores) | **Level on the principle, behind on the primitive set, ahead on regimes.** Nobody we read runs the same kernel body over in-RAM, mmap, streamed and compressed sources with identical results; but our drivers (`scan`, `vertices`, `frontier`) cover only the edgeMap core | See the gap list below; Ligra+ replicates our "compressed scans beat raw in parallel" result (D4) a decade earlier |
| **#44** framework features | see gap list | missing: bucketing, value-carrying frontiers, contention-free neighbour counting, edge-balanced tasks / split hub lists, sparse frontier array, automatic direction switch, VGC/hash bag, pack/filter | 3, 4, 5, 11, 12 |
| **D13** threads per kernel | Beamer: little gain from HT on 40 cores; IISWC'20: NWGraph/GKC gains in the "optimized" setting came almost entirely from 64 logical threads on 2 × 16 cores | **Level.** Per-kernel choice is what the optimized IISWC'20 runs did | But: our k-core's non-scaling past 8 threads is algorithmic (hub contention, per-round rescans — 33), not a memory-bandwidth class; capping its threads (m3-07 "investigate or cap threads") treats the symptom |
| **D14** deterministic by default | Per-kernel: GBBS distances/SCC labels deterministic, BFS parents and LDD not; GAP accepts any valid parent and tolerates PR noise from accumulation order; Galois async is nondeterministic. Community-style local moving: Mt-KaHyPar-SDet / DetJet | **Ahead in scope** (no system we read promises identity across memory budgets and regimes); **behind in cost** for Leiden (2.4–3.0× vs 16–24% for Mt-KaHyPar coarsening) | 21, 22 vs FINDINGS § B2; our lab uses **at least 256 sub-rounds per sweep at every level** (`lab/b2-leiden/src/main.rs`: `(n / 16384).next_power_of_two().clamp(256, 1024)`), Mt-KaHyPar a constant 16 (Louvain preprocessing) and 3 (clustering), with "fewer sub-rounds does not hurt quality, or even helps" (21 §4.4) |
| **#47** PageRank | cache blocking (41–43) for state ≫ LLC; Jacobi pull for small graphs | **Behind on Friendster-class graphs** (3.19 s/iter at 16 threads; GAP-pull-class), level on orkut-class | 41: 1.75× on friends; 42: 2.7–3.1× over hand-optimized pull on Twitter/RMAT27/SD |
| **#48** WCC | ConnectIt UF-Rem-CAS + k-out hybrid sampling | **Level with GAP Afforest** (orkut 12–17 ms, Friendster 0.36–0.50 s at 16 threads vs GAP-AF 73 ms / 0.306 s at 144 HT on a 4-socket box); **behind ConnectIt** by an unknown factor on our machine | 27: 1.67× (Friendster) to 8.6× (orkut) over GAP-AF at 144 HT. **Our "first neighbours" sampling is the "kout-afforest" variant that fails on locally clustered orders** (ClueWeb, Hyperlink2012) — relevant to web graphs and to #146 (relabel at ingest) |
| **#48** SCC | BGSS + hash bag + VGC | **Plan is behind**: FW-BW + trimming / coloring is the Multistep/iSpan family that 39 measures slower than sequential Tarjan on large-diameter graphs | 39 |
| **#49** BFS | DO-BFS α = 14, β = 24 (7; "α > 12 insensitive", β matters little); GAP keeps −degree in the parent array to get m_f without degree reads ("nearly 2×" on high-diameter graphs that stay top-down) | **Level** (α 15 / β 18 is inside the insensitive range); orkut 19–28 ms at 16 threads vs GBBS 12 ms at 144 HT | Risk for the road-graph acceptance: our `NodeSet::list_into` counts and lists **all n/64 bitset words** each round (worktree of #44, `edge_map/node.rs`); with thousands of levels on a road graph that is O(n/64 · D) — the per-round cost PASGAL and GAP avoid with sparse queues |
| **#49** SSSP | ρ-stepping / Δ*-stepping; Wasp asynchronous; bucket fusion (3.4× on road, 12); Δ from sampled weights (31) | **Plan is fragile**: plain Δ-stepping with a fixed Δ; 32 shows optimal Δ moves by orders of magnitude between weight distributions (mis-tuning up to 4.92× for Wasp, GBBS most robust) | 12, 29, 31, 32 |
| **#50** k-core | Liu et al. (33) | **Behind on Friendster/Twitter-class graphs**: ours 7.65–8.74 s on Friendster and 5.26–5.50 s on twitter-sym at 16 threads vs Liu 3.68 s / 2.72 s at 192 HT; **ahead or level on orkut** (0.28 s at 8 threads vs Liu 0.526 s at 192 HT; orkut needs 5,667 peeling rounds and Liu's self-speedup there is 7.49) | Our "PKC-style" is far better than the PKC Liu measured (Twitter 75.6 s), so their ratio over PKC does not transfer; the gain from active set + sampling must be measured |
| **#51** triangles | forward + merge; LOTUS when skewed; relabel only top-10% hubs (38) | **Level**: Friendster 67.5–75.8 s at 16 threads ≈ GAP 70.5 / GBBS 76.4 s at 32 SkyLakeX cores; merge vs hash confirmed (37: merge 1.3–2.5× faster) | SIMD + galloping will not move Friendster much (max degree 5K, 7.6% hub edges — 38) |
| **#52** centralities | MS-BFS bit-parallel batches; LAGraph batched Brandes (4 sources) beats GAP BC on 3 of 5 graphs | **Level in plan** | 13, 46; MS-BFS state is P × 3 × N × ω bits: Friendster with ω = 512 ≈ 12.6 GB per run (arithmetic) — must be budgeted |
| **#54** determinism CI | Mt-KaHyPar: 256 fixed chunks for random splits "regardless of number of threads"; counter-based RNG seeded per chunk | **Level/ahead** (CI across threads, oversubscription, wasm, regimes) | Add: every random choice through a counter-based RNG keyed by (seed, node/chunk) — Philox is integer-only and so bit-portable (26; inference) |
| **#18** det. Leiden scaling | see D14 row; adaptive windows (20, 24, DetJet prefix doubling) | **Behind** | the m1-04 hypothesis (tiny coarse sub-rounds, barriers) matches the code: ≥ 256 sub-rounds per sweep whatever the level size |
| **#22** memory bandwidth | PB / CSR segmenting / PCPM; reordering DBG +6.5% on "structured" graphs incl. Friendster (45) | **Not started** | Friendster pull here moves ~0.9 cache lines per edge in Beamer's measurement (3,285 M lines / 3,612 M edges); DPB 770 M |

**What GBBS (and Julienne, PASGAL, GraphIt) have that #44 lacks**, by value to our planned kernels:

| Primitive | Used by | Measured benefit | Determinism-safe? |
|---|---|---|---|
| Bucketing structure: lazy (Julienne, histogram updates, 128 open buckets + overflow) and eager thread-local with bucket fusion (GAP/GraphIt) | k-core, Δ-stepping, wBFS, set cover | k-core: Julienne 2.6–9.2× over scan-based peel (72 cores); lazy 4.3× better for k-core on Twitter, eager 43× better for SSSP on road (12) | yes if bucket contents are listed in a data-defined order (sort or fixed-block pack) |
| Contention-free neighbour counting (edgeMapSum, histogram) | k-core decrements, densest subgraph | 1.1–3.1× over fetch-and-add (4); ClueWeb k-core 62.9 vs 221 s | yes (integer) |
| Value-carrying frontiers (edgeMapData, vertexSubsetT) | SSSP, wBFS, k-core | enabling | yes |
| Edge-balanced tasks; one hub list split across tasks (Ligra+ chunks of T = 1000 with restart points; edgeMapBlocked) | every scan on skewed graphs, TC, BFS bottom-up | Ligra+: up to 2× loss without parallel decode of hub lists (40 cores); edgeMapBlocked 1.7× on ClueWeb wBFS | yes, if cut points come from a degree prefix sum (data only) |
| Sparse frontier array below a size threshold, dense bitset above | every traversal; essential on road graphs | PASGAL/GBBS road numbers above; GBBS coloring 1.2–1.6× slower "due to synchronizing on many rounds which contain few vertices" | yes (sort the sparse array, or keep insertion in a data-defined order) |
| Automatic direction switch helper (Ligra: |U| + Σdeg > |E|/20) | BFS, BC, radii, CC | one threshold "worked well across all applications" (1) | yes |
| Pull with early exit inside a list (decode a prefix) | bottom-up BFS, LPA | – (our #118 BFS slowdown 1.20–1.52× is partly whole-list decode) | yes |
| filter / pack (edge deletion, also on compressed lists) | TC orientation build, MSF, matching | enabling | yes |
| VGC (local search of ≥ τ vertices per task) + hash bags | SCC/CC reachability, k-core peeling on high-diameter graphs | SCC 4–19×, k-core up to 31.2× (GRID), 1.7–1.9× on roads (33, 39) | yes for reachability and peeling; **not for BFS levels** (breaks level order, 39) |

## 4. Challenges (ranked)

| # | Change | Expected gain (source, case) | Cost, risk against our promises | Cheapest experiment that settles it | Issue |
|---|---|---|---|---|---|
| 1 | **Deterministic Leiden: a small constant sub-round count** (16 or 32, a function of nothing) instead of ≥ 256 per sweep at every level; levels below a size threshold (a function of n) run on one thread | Mt-KaHyPar: constant 16 (Louvain) / 3 (clustering), quality insensitive or better with fewer; SDet 12.8× self-speedup at 16 threads; deterministic coarsening 16–24% over nondeterministic (21, 22). Our target ≤ 1.5× async at 16 threads looks reachable if barriers dominate (inference) | None in code: the lab binary has `--sub-rounds`. Risk: more oscillation with fewer sub-rounds — the smaller-label rule already handles it; quality must be checked | Interleaved A/B in `lab/b2-leiden`: `--sub-rounds` 8, 16, 32, 64 vs auto; lj and orkut; 8 and 16 threads; metrics: det/async time, Q, hash identical at 1–32 threads. ~1 hour | #18; then #57 |
| 2 | **Same-machine reference runs of GAP, GBBS, ConnectIt and Liu's k-core** (C++ tools in `bench/` only, never in the core), replacing NetworKit as the M3 bar | Answers "how far from best" for every kernel on *our* hardware; every cross-paper ratio in this report is from 2–4-socket machines | Build cost of four C++ repos outside the core; none in the product. Risk: none to promises | Build GAP + GBBS; run their BFS, CC, PR, k-core, TC, SSSP on lj, orkut, twitter-sym, Friendster at 16 threads under the harness, next to ours | #55 (perf tracking); M3 exit criterion "≥ NetworKit" → "≥ GAP reference, report vs GBBS" |
| 3 | **k-core after Liu et al.** — explicit active set (O(n + m), no per-round rescan), sampled degree counters for hubs (Las Vegas: exact, restart on detected error), VGC local queues of 128, hierarchical buckets | Friendster 3.68 vs PKC 33.1, Julienne 6.18 s; Twitter 2.72 vs 75.6 s (192 HT). Our implementation is already much better than their PKC, so expect less: unknown on our machine | Needs the bucketing primitive (#44). Core numbers are unique, so D14 holds whatever the schedule; sampling needs a counter-based RNG; peak memory unchanged (O(n)) | Lab A/B on twitter-sym and Friendster (and orkut as the hard case) at 8/16/32 threads; metric time, scaling past 8 threads, identical core numbers | #50 (replace "PKC-style"; drop "cap threads") |
| 4 | **Edge-map primitives before kernels freeze on it**: sparse frontier array for small frontiers (no O(n/64) bitset scan per round), edge-balanced task cuts from a degree prefix sum, bucketing (lazy + eager with fusion), value frontiers, direction-switch helper, prefix decode with early exit | Road BFS/SSSP: GAP/PASGAL/GraphIt numbers above (bucket fusion 3.4× on road, 12); hub splitting up to 2× (2) | Medium code in #44; all cut points data-defined, so D14 holds. **Format question to settle now**: splitting one hub list across tasks on the compressed default needs restart points inside long lists (Ligra+ restarts every 1,000 entries, encoding each chunk's first entry relative to the source) — decide in #28/#29 before the container spec freezes | Synthetic road graph (or roadNet-CA) BFS at 16 threads with the #44 driver: time per level vs a sparse-queue variant; twitter-sym PageRank and TC with node-range vs edge-balanced tasks | #44, #49, #28, #29 |
| 5 | **Propagation-blocked PageRank** for graphs whose state exceeds L3: push scan of the compressed adjacency in ascending source order into per-source-chunk bins of 64K destinations (f64 + u16 local id), accumulate bin by bin; bins bounded by the budget through source chunking | 1.75× on friends, 1.48× Twitter (41, 16 threads); our traffic estimate ~53 B/entry today vs ~15–20 B/entry binned (arithmetic, estimate) | **Deterministic by construction**: each destination receives contributions in ascending source order, independent of bin width and thread count, if bins are per fixed source chunk (not per thread) and read in chunk order; can be made bit-identical to the current pull if the accumulator reproduces the pull's 4-lane association (by position counter). Transient bins 10 B/entry (Friendster ≈ 36 GB unchunked) → must be chunked under a budget; wasm has no streaming stores | Zero-code first: DRAM bytes/entry of Friendster pull at 16 threads (AMD UMC counters). Then lab PB vs pull, interleaved, Friendster + twitter-sym + orkut (control), 16 threads, s/iteration and hash identity | #22, #47 |
| 6 | **WCC: UF-Rem-CAS finish + k-out hybrid sampling** (first neighbour + one hashed-random neighbour), no sampling when m/n < 3 | 1.67× Friendster, 1.86× Twitter, 8.6× orkut over GAP Afforest (27, 144 HT) — ratio on our machine unknown | Small. Min-root linking keeps the minimum-id label, so D14 unchanged; the random neighbour must come from a counter-based hash of (seed, node). Removes the failure of first-k sampling on locally clustered orders (web crawls; any community order from #146) | Lab A/B on orkut, Friendster, uk-2005 (web, clustered order) at 16 threads; metric time and label hash | #48 |
| 7 | **SCC: BGSS** (random-permutation prefix-doubling multi-reachability, phase-concurrent hash table, VGC) instead of FW-BW + trimming / coloring; relabel to the minimum id per SCC in one O(n) pass | 1.7× Twitter, 3.1× LJ over GBBS; 4–19× on large-diameter graphs; Multistep 5.8× slower on Twitter (39, 192 HT) | Medium code; needs a deterministic hash table (19: 7–23% over nondeterministic); partition unique, label fixed by the min-id pass | twitter-2010 directed and a lattice/road digraph vs a simple FW-BW, 16 threads | #48 |
| 8 | **SSSP: Δ*/ρ-stepping with bucket fusion and a Δ chosen from sampled weights** (AdaMW's rule, made thread-independent), leaf skipping; distances deterministic, parents by a post-pass (cheapest, earliest edge — already the m7-02 rule) | 1.46× (Friendster) to 2.6× (Twitter) over GAP Δ-stepping (29, 192 HT); robustness: 32 | Medium. AdaMW's Δ uses the thread count T — replace by a data-only rule so the *speed* is the only thing that varies (distances are unique anyway). Asynchronous Wasp is compatible with D14 for distances (writeMin; inference) but not needed first | Natural-weight graphs (OSM road, a weighted social graph) vs uniform weights, Δ sweep, 16 threads | #49, #91 |
| 9 | **Triangles: LOTUS-style hub path gated by skew** and relabelling of only the top-10% hubs | Twitter 3.2× over GAP, Friendster 1.24× (38, 32 cores) | Extra 256 MB hub bit-array (budget it); integer counts deterministic | twitter-sym and Friendster, 16/32 threads, vs current forward-merge | #51 |
| 10 | **Counter-based RNG for every seeded choice** (sampling in WCC, k-core, betweenness; LPA ties; Leiden sub-round tags) | Thread-count-independent random streams by construction (26) | Tiny; integer-only arithmetic, portable to ARM and wasm (inference) | Unit test: same streams at 1–32 threads and on wasm32 | #54 |

Not ranked high, but checked: **Demmel–Nguyen binned summation** would let us drop fixed
reduction blocks (sum independent of order and partition) at 3–7× the cost of a plain sum
(25); fixed blocks are already platform-independent under IEEE round-to-nearest and far
cheaper — keep them, and keep fixed-point totals for modularity.

## 5. Our novelty claims, checked

| Claim | Verdict |
|---|---|
| One kernel body over raw, compressed, mmap and streamed sources, identical results | **Not found** in what we read. Ligra+ does compressed vs raw in memory; GridGraph/FlashGraph (previous survey) stream; none promises identical bytes across them |
| Identical results across memory budgets and execution regimes (D14 widened) | **Not found.** Deterministic Galois promises "portable and parameterless" (same output on any machine and for any machine-dependent scheduling parameter) — the same idea for threads and machines, not for out-of-core regimes |
| Compressed adjacency faster than raw for parallel scans (D4) | **Not new**: Ligra+ (2015) measured byte-RLE 14% faster than uncompressed on average in parallel, 3% slower sequential (40 cores). We replicated it |
| Fixed, data-defined reduction blocks for reproducible floats | **Not new**: Mt-KaHyPar's 256 chunks "regardless of number of threads"; standard reproducible-reduction practice |
| Decoding a batch of lists before the kernel body (#118) | **Not found** as such; Ligra+ decodes per list or per chunk of a hub list. Small, plausible novelty |
| Min-id hooking gives deterministic WCC labels for free | **Not new**: Afforest's invariant π(x) ≤ x, ConnectIt's link-to-smaller-root |
| Deterministic Leiden with sub-rounds | **Not new**: Mt-KaHyPar-SDet deterministic Louvain (2022) with sub-rounds, sorted volume updates; ours uses a commutative fixed-point commit instead of their sort, which is cheaper (inference) |
| Branded node indices (`Node<'g>`, GhostCell-style) to drop bounds checks safely in kernels (#44 worktree) | **Not found** in graph frameworks; the technique is from GhostCell (cited by Rusty-PBBS, 47). Rusty-PBBS reports irregular writes in Rust need atomics or `unsafe`; our design answers it |

## 6. What to ignore, and why

- **Gauss–Seidel / asynchronous PageRank** (IISWC'20 winner on road): order-dependent, breaks D14. A block-Jacobi variant (Gauss–Seidel inside fixed blocks, Jacobi across) would be deterministic — unmeasured, low priority.
- **Galois-style asynchronous worklists and OBIM soft priorities**: nondeterministic schedule; their wins are on road SSSP/BFS, which bucket fusion and sparse frontiers recover deterministically.
- **Deterministic Galois' generic DIG scheduler**: median 4.2× over nondeterministic; hand-written bulk-synchronous determinism is cheaper.
- **Deterministic reservations for Leiden commits**: community totals are shared by all members; big communities serialize (24 notes the same with the largest component).
- **Demmel–Nguyen binning in hot loops**: 3–7× a plain sum; fixed blocks suffice.
- **DPB's persistent destination copy**: 2.12 vs 2.16 s over plain PB on friends for 4 B/edge more memory.
- **Gorder**: 254.9 PageRank iterations to amortize on Friendster (45).
- **DBG / degree sorting alone**: +6.5% on "structured" graphs incl. Friendster (45); Sort/HubSort up to −38%.
- **Hash-based triangle intersection**: 1.3–2.5× slower than merge (37).
- **Nibble / bit-level codes for kernels**: 35–73% slower sequentially (2); bit-level stays in the webgraph archival tier (D5).
- **Label-propagation connectivity**: 478–590× slower on road (9, 27).
- **GraphBLAS as an engine**: C dependency, 64-bit indices, 1.1–220× slower than GAP on most GAP kernels (13).
- **NUMA partitioning** (GraphIt, Galois, Optane study): single-socket target; at most a CCD-aware tweak.
- **VGC for BFS levels and parents**: breaks level order (39); use it for reachability and peeling only.
- **Batch-dynamic k-core** (34), **UpDown / processing-in-memory** work: different problem or hardware.
