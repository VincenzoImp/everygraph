# Memory-bounded graph algorithms: a survey for Everygraph

Survey of 2026-11-25 (web research, Opus medium, three parallel searches; primary sources
opened unless marked "snippet"). It answers: which algorithms designed for limited memory
could Everygraph use or learn from, per kernel class, and how each fits the plan and the
decisions (D11 regimes, D14 determinism, "What 'repeatable' promises" in
`design/DECISIONS.md`). Recommendations are inputs to the issues they name; the issues
decide. Licences marked "not verified" must be checked before any code is borrowed, and a
reimplementation of a published method goes through the patent check (#10, #122).

Literature survey for Everygraph: algorithms built for limited memory, per kernel class

How I worked: I split the search across three parallel forks (community detection / CC, BFS, k-core, triangles / centrality, PageRank, layout, systems). Each one opened the primary pages; anything it only saw as a search snippet is marked "(snippet)". I read the issues that matter: m1-02, m4-04, m4-09, m1-04, m3-03, m3-05 to m3-09, m5-01, m7-02, m7-03 and m7-07. I also read ADR 0011, the B2 section of FINDINGS, and the DECISIONS row on what "repeatable" promises. Nothing in the repo was edited, built or run.

Bottom line: nobody has published an external-memory Leiden that reaches in-RAM quality. The closest is CluStRE, at Q 0.5871 on Friendster with O(n) memory. For level 0, the problem is self-inflicted: local moving reads only adj(u) plus state that is already in RAM. The graph is read at random only because of the visiting order (ADR 0015's block permutation, plus the pruning queue). Fix the order, keeping it deterministic and independent of the budget, and level 0 becomes sequential.

Three facts frame everything below:
- u32 ids keep per-node state small (a visited bitmap is at most 0.5 GiB). So semi-external algorithms (nodes in RAM, edges on disk) are the right model. Fully external ones (Munagala–Ranade and Mehlhorn–Meyer BFS, Contract & Expand) are not needed.
- Everything recommended "adopt" is exact and deterministic, so it does not conflict with D14 or with "same result for any memory budget".
- Everything approximate must be a labelled variant, and the planner must never pick it on its own.

=====================================================================
1. TABLES PER KERNEL CLASS
=====================================================================

Columns: algorithm (source) | guarantee | memory / I/O | deterministic? | evidence | implementation, licence | fit (issue) | recommendation

A. Community detection

1. Ideas from memory-efficient KaMinPar / TeraPart (Salwasser, Seemaier, Sanders, arXiv 2410.19119, 2024; https://arxiv.org/html/2410.19119)
   - Guarantee: partitioning heuristic, not modularity.
   - Memory / I/O: in RAM. Label-propagation clustering and contraction need O(n + p·T) auxiliary memory instead of O(np), using two-phase "bumped" hashing. The coarse CSR is written in one pass. High-degree neighbourhoods are split into chunks of 1,000; the graph is stored with gap + VarInt + intervals.
   - Deterministic? Not discussed.
   - Evidence: contraction auxiliary memory on webbase2001 fell from 6.0 to 1.4 GiB. Compression costs about 6%. eu-2015 (80.5 B edges) took 77 s in 87 GiB.
   - Implementation: KaMinPar, MIT (snippet). A GPL-header issue was fixed in October 2026 (snippet).
   - Fit: m1-02-ooc-aggregation, m4-03-parallel-aggregation.
   - Recommendation: ADOPT the techniques (bumped two-phase hashing, one-pass coarse CSR, chunked high-degree neighbourhoods).

2. CluStRE (Chhabra, Peretz, Schulz, SEA 2025, LIPIcs 338; https://arxiv.org/abs/2502.06879)
   - Guarantee: heuristic. Modularity is kept exactly on a quotient graph.
   - Memory / I/O: Θ(n) assignments plus community volumes. Light mode is one pass; Light+ re-streams only the active nodes in file order. The quotient-graph hash map has no stated bound.
   - Deterministic? Light and Light+ yes, for a fixed stream order. The memetic modes are time-limited, so no.
   - Evidence: Friendster Q 0.5871 (Strong mode); in their setup Louvain and VieClus failed. uk-2007-05 (3.3 B edges): Q 0.8789 at 2.18 GB peak. On average 96.8% of Louvain's Q at about 18% of its memory. No runtimes for the large graphs.
   - Implementation: KaHIP/CluStRE, MIT (repo opened).
   - Fit: m4-09-label-propagation (a much better preview than plain LPA); a labelled fallback for m4-04-ooc-leiden at very small budgets.
   - Recommendation: PROTOTYPE IN M1 the Light+ "re-stream only active nodes" I/O pattern. Never substitute it silently for Leiden.

3. Graphyti semi-external Louvain on FlashGraph (Mhembere, Zheng, Priebe, Vogelstein, Burns, arXiv 1907.03335, 2019; PDF read)
   - Guarantee: Louvain.
   - Memory / I/O: O(n) in RAM, edges on SSD. It never writes the coarse graph: each level re-reads the level-0 edges through a vertex-to-community map and uses lazy deletion.
   - Deterministic? Not stated.
   - Evidence: Twitter, 1.5 B edges, at most 4 GB of memory. 2× faster than a version that rewrites the graph onto a RAM disk; the gain is in the early levels.
   - Implementation: flashxio/graphyti, licence not verified.
   - Fit: m1-02-ooc-aggregation. It is the alternative to spilling the level-1 graph.
   - Recommendation: PROTOTYPE IN M1 (idea L4 in section 2).

4. Buffered streaming multilevel: HeiStream (Faraj, Schulz, ACM JEA; arXiv 2102.09384) and BuffCut (SEA 2026, arXiv 2602.21248) — both snippet only
   - What it does: runs multilevel on a RAM-sized buffer of consecutive nodes plus the already-assigned blocks.
   - Fit: m4-04-ooc-leiden. This is the pattern behind the windowed order (idea L2).
   - Recommendation: LATER.

5. SCoDA (Hollocou, Maudet, Bonald, Lelarge, arXiv 1703.02955, 2017)
   - Guarantee: none on modularity. There is a conductance argument, assuming a random edge order.
   - Memory / I/O: 2 integers per node, one pass.
   - Deterministic? Yes, for a fixed order.
   - Evidence: Friendster in 314 s with 1.1 GB. CluStRE measured its Q at 0.0334.
   - Implementation: ahollocou/scoda, GPL-3.0.
   - Fit: m4-09 at most, as a baseline.
   - Recommendation: NO. Do not copy the GPL code.

6. Weighted Misra–Gries sketch in place of per-thread tables (Sahu, arXiv 2411.02268)
   - Guarantee: heuristic.
   - Memory: about 0.5 KB per thread.
   - Deterministic? Inherits the base algorithm.
   - Evidence: up to about 1% lower Q; Leiden 3.15× slower.
   - Implementation: puzzlef repository, licence not checked.
   - Fit: none. It attacks memory we do not spend: our O(n) state is already 21–53 B/node.
   - Recommendation: NO.

7. GVE-Leiden / GVE-LPA (Sahu, ICPP 2024; arXiv 2312.13936)
   - Guarantee: Leiden. Space O(T·n + m).
   - Deterministic? No.
   - Evidence: 403 M edges/s on sk-2005 (3.8 B edges, 9.4 s).
   - Implementation: puzzlef/leiden-communities-openmp, MIT.
   - Fit: reference for m4-01-leiden-production.
   - Recommendation: REFERENCE only.

8. NetworKit PLM/PLP, Grappolo, Vite
   - All in RAM or distributed.
   - Recommendation: REFERENCE only.

9. Reordering at ingest
   - Recursive graph bisection, BP (Dhulipala et al., KDD 2016; arXiv 1602.08820, snippet): reorders for compression and locality, at billions of nodes. Fit: m2-03-adjacency-codec, m2-08-csr-build. Recommendation: LATER, as an ingest option with a fixed seed.
   - Gorder (MIT): reordering is too slow at 10^9 edges. Recommendation: NO.
   - Rabbit Order: the NTT licence is evaluation-only (no modification, no redistribution, no commercial use). Recommendation: NO for the code; a clean reimplementation would need m0-10-patent-check.

10. Label propagation out of core
   - No paper found. Every LPA paper I found runs in RAM.
   - m4-09's plan (synchronous, ties broken by hash, streamed) is sound.

B. Connected components and SCC

1. One-scan semi-external union-find (Abello, Buchsbaum, Westbrook, ESA 1998 / Algorithmica 2002; https://link.springer.com/article/10.1007/s00453-001-0088-5)
   - Guarantee: exact.
   - Memory / I/O: O(n) memory, one scan.
   - Deterministic? Yes, with min-id roots.
   - Evidence: already in the lab (Friendster 3.55 s).
   - Fit: m3-05-components.
   - Recommendation: ADOPT (already planned).

2. 1PSCC / 2PSCC, semi-external SCC (Zhang, Yu, Qin, Chang, Lin, SIGMOD 2013; VLDB J. 2015)
   - Guarantee: exact.
   - Memory: about c·n with c ≈ 3 (snippet).
   - Deterministic? Yes, after canonicalising labels to min id.
   - Implementation: no code found.
   - Fit: m3-05-components.
   - Recommendation: LATER. twitter-2010 fits in RAM.
   - Practical path when SCC is built: trimming, then forward-backward on the giant SCC, then min-label colouring for the rest. All steps are scans with O(n) state on the edge-map (m3-01/m3-03), and they are deterministic. This is our own synthesis, not a published system.

3. Contract & Expand (ICDE 2014) and Wan & Wang semi-external SCC (TKDE 2023)
   - Content not verified.
   - Recommendation: read before building out-of-core SCC.

4. GraphZeppelin, sketch-based CC (SIGMOD 2022; arXiv 2203.14927)
   - Probabilistic, for dynamic streams.
   - Recommendation: NO for static graphs; perhaps m9-04-temporal-segments.

C. BFS, shortest paths, DFS

1. Munagala–Ranade BFS (SODA 1999) and Mehlhorn–Meyer BFS (ESA 2002)
   - Guarantee: exact, fully external.
   - I/O: MR is O(n + sort(n+m)) (via Wikipedia; primary paper not opened). MM's bound is not verified.
   - Evidence: Ajwani et al. (ALENEX 2007, snippet) report about 1 B edges in "a few hours" to "days".
   - Recommendation: NO. Not needed at u32, and orders of magnitude slower than the semi-external design.

2. EP-BFS (Wan, Han, arXiv 2507.12925, 2025, not peer reviewed)
   - Guarantee: exact semi-external BFS; a spanning tree is held in RAM.
   - Evidence: up to eu-2015 (91 B edges).
   - Implementation: no code.
   - Fit: m3-06-traversal.
   - Recommendation: LATER, for ideas only.

3. EP-DFS (Wan, Wang, Inf. Sci. 2022) and D&C DFS (SIGMOD 2015)
   - Recommendation: NO. No kernel that needs DFS is planned.

4. Our own idea for m3-06-traversal, bottom-up out of core
   - Decode only the lists of unvisited nodes, skipping fully visited blocks with the Elias–Fano offsets.
   - For top-down steps, sort the frontier and coalesce reads (FlashGraph, section F).

D. k-core

1. SemiCore* (Wen, Qin, Zhang, Lin, Yu, "I/O efficient core graph decomposition at web scale", ICDE 2016 Best Paper / TKDE 2018; https://arxiv.org/abs/1511.00367, PDF read)
   - Guarantee: exact. Local h-index updates run to a unique fixpoint.
   - Memory / I/O: O(n) memory (two arrays); O(l·(m+n)) read-only I/O over l iterations. Only nodes whose estimate may have changed are re-read.
   - Deterministic? Yes (unique fixpoint).
   - Evidence: Clueweb (978 M nodes, 42.6 B edges) in under 4.2 GB. l = 62 on Twitter, 2137 on UK. All graphs except Clueweb finished in under 10 minutes on 16 GB RAM and a 7200 rpm disk.
   - Implementation: no code found.
   - Fit: m3-07-kcore-degrees, which has no out-of-core path today.
   - Recommendation: ADOPT for the semi-external regime (M3).

2. EMCore (Cheng et al., ICDE 2011), measured by Khaouid et al. (PVLDB 9(1) 2015; https://www.vldb.org/pvldb/vol9/p13-khaouid.pdf)
   - Guarantee: exact.
   - Memory: not bounded; in the worst case it loads the whole graph.
   - Evidence: twitter-2010 took 22,593 s at 8 GB and could not run at 4 GB. The in-RAM Batagelj–Zaversnik algorithm on WebGraph-compressed data took 333 s.
   - Implementation: mgbarsky/emcore, MIT.
   - Recommendation: NO. But this is strong outside evidence for D4/D11: compressed in RAM beats partitioned external memory by about 70×.

E. Triangles and clustering coefficient

1. MGT (Hu, Tao, Chung, SIGMOD 2013; https://www.cse.cuhk.edu.hk/~taoyf/paper/sigmod13-triangle.pdf, PDF read)
   - Guarantee: exact listing.
   - Memory / I/O: O(m²/(M·B) + K/B), where K is the number of triangles. Θ(m/M) passes: load a chunk of the oriented graph, stream the rest, intersect.
   - Deterministic? Yes.
   - Evidence: up to WebUK (939 M edges), with memory at 5–25% of the graph's size; more than 10× faster than competitors.
   - Implementation: authors' code, licence not verified.
   - Fit: m3-08-triangles. Per-node u64 counts fit in O(n). The number of passes is about compressed forward size / budget.
   - Recommendation: ADOPT as the semi-external path (M3).

2. PDTL (Giechaskiel, Panagopoulos, Yoneki, ICPP 2015; PDF read)
   - Guarantee: exact; parallel and distributed MGT.
   - Evidence: more than 6 B edges.
   - Implementation: code location not verified.
   - Fit: m3-08-triangles.
   - Recommendation: take its parallel orientation and edge-range split when MGT is built.

3. Pagh–Silvestri (PODS 2014; arXiv 1312.0723)
   - Guarantee: exact.
   - I/O: optimal, O(m^1.5/(√M·B)).
   - Evidence: no experiments.
   - Recommendation: NO.

4. Wedge sampling (Seshadhri, Pinar, Kolda, SDM 2013; arXiv 1309.3321)
   - Guarantee: approximate (ε, δ) for global, local and degree-wise clustering.
   - Memory: O(n) prefix sums plus random probes.
   - Deterministic? Seeded, but not equal to the exact value.
   - Fit: m3-08-triangles.
   - Recommendation: LATER, only as a labelled `approx` variant.

5. DOULION (KDD 2009)
   - Recommendation: NO; wedge sampling dominates it.

6. TRIÈST (KDD 2016; arXiv 1602.07424)
   - One-pass reservoir sampling for streams.
   - Recommendation: LATER, for temporal or live mode.

Side result: the reference counts m3-08 asked to verify match SNAP (lj 177,820,130; orkut 627,584,181; Friendster 4,173,724,142). SNAP gives Friendster's average clustering coefficient as 0.1623.

F. Distances, neighbourhood function, centralities

1. HyperANF / HyperBall (Boldi, Rosa, Vigna, WWW 2011, arXiv 1011.5599; Boldi, Vigna, arXiv 1308.2144)
   - Guarantee: approximate neighbourhood function, harmonic and closeness centrality, via HyperLogLog.
   - Memory / I/O: about a dozen bytes per node; one sequential scan per hop.
   - Deterministic? Yes, with a fixed hash. A union is a per-register max, so the bytes are identical for any number of threads, any chunking and any regime.
   - Evidence: "billions of nodes in a few hours" (abstract).
   - Implementation: webgraph-algo, Rust, Apache-2.0 OR MIT.
   - Fit: m3-09-centrality; diameter and distance distribution.
   - Recommendation: ADOPT as an explicitly approximate kernel. It is the best fit for the "repeatable" rule in this class.

2. Riondato–Kornaropoulos betweenness (WSDM 2014 / DMKD 2016)
   - Guarantee: additive ε for all nodes with probability 1−δ. The sample count depends on vertex diameter.
   - Deterministic? Yes, with a seeded RNG and a fixed sample count.
   - Implementation: NetworKit, MIT.
   - Fit: m3-09-centrality.
   - Recommendation: ADOPT as the baseline.

3. KADABRA (Borassi, Natale, ESA 2016; arXiv 1604.08553)
   - Guarantee: absolute error with adaptive stopping.
   - Evidence: claimed about 100× faster than RK (snippet).
   - Deterministic? Only if the stopping test runs at fixed batch boundaries.
   - Implementation: NetworKit, MIT.
   - Recommendation: LATER.

4. SILVAN (arXiv 2106.03462) — LATER. ABRA — NO.

5. Multi-source BFS with bitsets (Then et al., PVLDB 8(4) 2014)
   - Guarantee: exact.
   - Use: batching the betweenness samples so each batch costs one pass per BFS level.
   - Fit: m3-06-traversal, m3-09-centrality.
   - Recommendation: PROTOTYPE.

6. Closeness
   - Cohen et al. (COSN 2014; arXiv 1409.0035) and exact top-k (Bergamini et al., ALENEX 2016) need random access.
   - Recommendation: LATER, in RAM only; HyperBall covers the out-of-core need.

G. PageRank

1. Semi-external Jacobi iteration
   - Guarantee: exact, deterministic with fixed reduction blocks.
   - Evidence: already measured in the lab; FlashGraph reports up to 80% of in-memory speed with a 1 GB cache.
   - Fit: m3-04-pagerank.
   - Recommendation: ADOPT as planned.

2. Gauss–Seidel / cross-iteration reuse: LUMOS (ATC 2019), CLIP (ATC 2017)
   - Fewer passes, but the floating-point order changes, so the results are not identical across regimes.
   - Recommendation: NO as a substitute; at most a labelled variant used in every regime.
   - *Note 2026-11-25 (coordinator, after the first research wave; `docs/research/README.md` A9).* This "no" holds for order-sensitive kernels only: PageRank sums, Leiden, LPA. LUMOS claims synchronous semantics (see the addendum). Monotone fixpoint kernels (min-label WCC, BFS depth, SSSP distances, k-core) have a unique answer, so order-free execution and skipping inactive lists are allowed for them.

3. Personalized PageRank: FORA (KDD 2017; arXiv 1908.10583), PowerPush/SpeedPPR (arXiv 2101.03652)
   - Random access.
   - Recommendation: LATER, in RAM. For out of core, prefer personalized power iteration over the existing scan.

H. Layout (the project's own disc packing stays the core; these matter for leaf placement)

1. SNAP-tFDP (IEEE VIS 2026; arXiv 2608.01907)
   - Method: O(m) edge-centric negative sampling.
   - Evidence: 72% less memory than baselines; 4 M nodes in under 10 s on GPU.
   - Licence: not verified.
   - Fit: m5-01-layout.
   - Recommendation: PROTOTYPE for large leaf communities.

2. DRGraph (TVCG 2020; arXiv 2008.07799) and t-FDP (arXiv 2303.03964)
   - Recommendation: REFERENCE.

3. BatchLayout (PacificVis 2020; arXiv 2002.08233)
   - Batch-synchronous, so deterministic.
   - No licence, so borrow the idea only.

4. FM3 (OGDF, licence unclear), sfdp (Graphviz, EPL-2.0), OpenOrd, ForceAtlas2 (GPL in Gephi, MIT in graphology)
   - Use as quality baselines only.
   - OpenOrd's coarsen-by-clustering idea is already subsumed by the project's community-hierarchy layout.

I. Out-of-core systems, and whether their lessons hold on a compressed adjacency

1. FlashGraph (Zheng et al., FAST 2015; PDF read)
   - What it did: vertex state in RAM, only the requested edge lists read from SSD, adjacent requests merged.
   - Evidence: 129 B edges with a 4 GB cache: BFS 298 s, WCC 461 s, PageRank 2041 s, triangles 7818 s. With a 1 GB cache, at least 65% of in-memory speed.
   - Licence: Apache-2.0.
   - Lesson for us: sort the frontier and coalesce compressed byte ranges, which the Elias–Fano offsets give cheaply. It works unchanged on our format.
   - Fit: m3-01-edge-map, m7-03-ooc-queries.
   - Recommendation: ADOPT the pattern.

2. Graphene (FAST 2017)
   - What it did: a bitmap-driven asynchronous I/O model.
   - Fit: m3-03-semi-external (io_uring).
   - Recommendation: ADOPT the idea.

3. Sage (Dhulipala et al., VLDB 2020; arXiv 1910.12310)
   - What it did: a read-only graph plus O(n) writable memory, which is exactly our model.
   - Evidence: 128 B edges on Optane, close to DRAM-only systems.
   - Implementation: GBBS, MIT.
   - Recommendation: ADOPT as the algorithm reference.

4. GridGraph (ATC 2015)
   - What it did: a 2D edge grid and skipping of inactive blocks. Apache-2.0.
   - Lessons: block skipping maps onto an activity bitmap plus the offsets. A 2D grid would let PageRank and WCC run when even the O(n) state does not fit, byte-identical if the accumulation order is fixed.
   - Fit: block skipping in m3-01-edge-map; the 2D grid in m9-07-u64-ids-server-scale.
   - Recommendation: block skipping now; 2D grid later.

5. Blaze (SC 2022)
   - What it did: online binning by destination with no atomics.
   - Licence: none (the repository has no licence file, as the review of 2026-11-26 found).
   - Lesson: push reductions without atomics. **The bins do not give deterministic float sums** (corrected 2026-11-26, [sota-review-2026-11.md](sota-review-2026-11.md)): scatter threads fill bins at the same time, and a full bin goes to whichever gather thread takes it (Blaze dissertation § 5.3.1), so the order in which a destination's values are added depends on the schedule.
   - Recommendation: read only; do not build on it for deterministic kernels.

6. Ligra+ (DCC 2015)
   - Evidence: compressed is slightly faster in parallel. This agrees with the project's own finding (#118).
   - Recommendation: evidence only.

7. Mosaic (EuroSys 2017, MIT)
   - What it did: Hilbert-ordered tiles with 16-bit local ids.
   - Lesson: relevant to cache blocking.
   - Fit: m1-08-memory-bandwidth.
   - Recommendation: LATER.

8. GraphChi (Apache-2.0) and X-Stream (Apache-2.0)
   - GraphChi rewrites mutable edge shards; X-Stream streams all edges even for a small frontier.
   - Recommendation: NO.

9. CLIP, LUMOS, ACGraph (SIGMOD 2026; arXiv 2511.07886)
   - They get their I/O savings from asynchronous or reordered updates, which conflicts with D14.
   - Recommendation: NO. Say openly that we give these savings up.
   - *Note 2026-11-25 (coordinator, after the first research wave; `docs/research/README.md` A9).* Narrowed: the conflict with D14 is real only for order-sensitive kernels (PageRank sums, Leiden, LPA). For monotone fixpoint kernels (min-label WCC, BFS depth, SSSP, k-core) the output is the unique fixpoint whatever the order. Asynchronous, on-demand and pruned execution is allowed for them. LUMOS is synchronous (addendum, "One correction to the survey above").

=====================================================================
2. IDEAS FOR THE OUT-OF-CORE LEIDEN LEVEL-0 BOTTLENECK
=====================================================================

All five keep Leiden exact. For L1 and L2, the visiting order must be a fixed parameter, never derived from the budget; otherwise the same-result-for-any-budget promise breaks.

L1. Get the decorrelation from a seeded relabelling at ingest (random, or BP order), then visit in storage order.
- Why: ADR 0015's gain comes from breaking the correlation with id order, not from new randomness on each pass. Level 0 then becomes one sequential stream per pass.
- Costs: results must be mapped back through the id dictionary (m2-07-id-mapping), and a random relabelling probably hurts compression.
- Cheap test: shuffle the ids of lj and orkut once, run Leiden in id order, and compare Q, community count and time with block-permuted order on the original ids. Minutes in RAM. Also measure the compressed size after the shuffle.

L2. Windowed permutation.
- What: permute blocks inside a window of fixed size W (say 1k–64k nodes) that slides in disk order. Each window is read once, sequentially. This generalises m4-04's "raise blocks to about 1k and schedule them in disk order".
- Cheap test: on orkut, sweep W from 1 block up to the whole graph, recording Q and the bytes read. ADR 0015 already shows no quality loss up to blocks of 1,024.

L3. Sweep the active set (pruning queue) as a bitmap in disk order, with block skipping.
- What: replace the FIFO queue with a bitmap swept in disk order, skipping blocks with no active nodes (GridGraph, CluStRE Light+). Refinement only looks at neighbours inside the node's own community, so it can also run in vertex order.
- Cheap test: on lj and orkut, log the fraction of active blocks per sweep and the extra moves against FIFO; check Q is unchanged within noise. Use the same log to replay the experiment's access trace with FlashGraph-style range coalescing, and count bytes read and requests issued.

L4. Never materialise level 1 (Graphyti-style): stream the level-0 edges through community[].
- Why: level 1 keeps 22–44% of the entries, so spilling and re-reading it may cost about as much I/O as k re-streams of level 0, without the build.
- Cheap test: on orkut, count the level-1 local-moving passes, then compare (a) bytes to spill and read with (b) k × level-0 compressed size. Decide before tuning the spill path.

L5. TeraPart's bumped two-phase hashing and one-pass coarse CSR write in the sort-based aggregation.
- What: fixed-size thread tables, with high-degree vertices bumped to a second phase. Memory stays bounded and fits the accountant (m2-04-arrays-accountant).
- Cheap test: peak anonymous memory of level-0 aggregation on Friendster in RAM, with and without bumping.

=====================================================================
3. DECISIONS OR TARGETS THAT LOOK WRONG OR IMPROVABLE
=====================================================================

1. ADR 0015 (block-permuted order of 64-id blocks) is itself the cause of random access out of core. Restate it as "an order decorrelated from ids, with locality kept at the storage granularity, and the window fixed independently of the budget". Otherwise m4-04's target (at most 2.5× the in-RAM time at 16 GiB) fights the decision. m4-04's "schedule blocks to follow disk order within a pass" breaks the any-budget identity if the schedule depends on the budget. The order must be a fixed function of the graph and the seed.

2. m3-07-kcore-degrees and m3-08-triangles have no bounded-memory path, which leaves the never-run-out-of-memory promise uncovered for them. SemiCore* and MGT are exact and deterministic. Add them, or have the planner refuse explicitly.

3. m3-09-centrality treats HyperLogLog and multi-source BFS as interchangeable. HyperBall is strictly better for repeatability (bit-stable across budgets, scan-shaped). Keep multi-source BFS for betweenness sampling. KADABRA-style adaptive stopping must check only at fixed batch boundaries, or m3-11-determinism-ci will fail. The "approximate betweenness on orkut in under 60 s" target has no outside support.

4. m3-01-edge-map's random-access path says only "prefetch hints". It should specify sorted-frontier, coalesced range reads (FlashGraph, Graphene) on top of the Elias–Fano offsets.

5. D11's rule "if the O(n) state does not fit, refuse" could be relaxed later with 2D grid streaming, which keeps results identical. This is low priority at u32 and matters for m9-07-u64-ids-server-scale.

6. m1-04-det-leiden-scaling cites LD-Leiden (arXiv 2502.18497) as determinism literature, but it is a dynamic method that is not deterministic. The citation looks misplaced; Mt-KaHyPar remains the relevant reference.

7. m4-09-label-propagation: CluStRE Light+ (MIT) reaches Q 0.5871 on Friendster in O(n) memory, close to Leiden's 0.60. It could make a far better labelled preview, or low-budget fallback, than plain LPA.

8. Outside evidence supports D4/D11: Khaouid et al. find compressed in RAM about 70× faster than EMCore, and Ligra+ finds compressed slightly faster than raw in parallel.

=====================================================================
4. NOT FOUND OR NOT VERIFIED
=====================================================================

- No external-memory or semi-external Leiden reaching in-RAM quality, and no out-of-core label-propagation paper; none found.
- CluStRE gives no runtimes on the large graphs.
- Licences not verified: Graphyti, Vite, puzzlef's leiden-lowmem, MGT code, STXXL, Graphene, Aspen, BatchLayout, DRGraph, SNAP-tFDP, t-FDP. OGDF returned NOASSERTION.
- Unknown whether KaMinPar has a deterministic mode.
- Seen only as search snippets: BP reordering details, BuffCut, HeiStream, KADABRA's speed-up figures, Ajwani et al.'s BFS timings.
- Bounds not read: the exact I/O bounds of Abello et al., 1PSCC and Mehlhorn–Meyer; and the contents of Contract & Expand and Wan & Wang's SCC paper (pages blocked).
- No code found for SemiCore* or 1PSCC. EP-BFS's baselines and absolute times not verified.
- Venue of PowerPush, and HyperBall's pass count on Friendster, not verified.
- No figures beyond their abstracts for GridGraph, Graphene, LUMOS, GraphChi or X-Stream.
- NetworKit PLM's "3.3 B edges" figure not confirmed.
- The ACM out-of-core graph-processing survey (DOI 10.1145/3838595) returned 403 and was not read.

## Addendum (2026-11-25): a second list, verified

The owner received a second list of 18 papers and 4 resources from another assistant. Each item was checked
on 2026-11-25 (web research, Opus; nothing built or run). Bibliographic details were confirmed
against Crossref (`api.crossref.org/works/<doi>`) and the publisher or arXiv page. "Read" means the full text was opened:
the Seraph, Planar, UMap and LDBC Graphalytics PDFs, and the MMap paper on PMC. "Abstract"
means only the abstract was opened (through ACM, Semantic Scholar or arXiv). ACM full texts
returned 403. Statements about determinism are our own inference from the paper's execution
model, not a claim made by the authors.

**Verdict on the list.** Every item exists. None is invented. Five carry small errors:

| Item | As cited | Correct |
|---|---|---|
| MMap | "2014/2015" | IEEE BigData **2014**, pp. 159–164, doi 10.1109/BIGDATA.2014.7004226. 2015 is only the PMC manuscript. |
| Planar | "PVLDB vol 18, 2025" | PVLDB 18(3):756–769, issue dated **Nov 2024**; the paper's own reference format says 2024. It was presented at VLDB 2025, so this is a minor slip. |
| Kaleido | "ICDE 2020, arXiv 1905.09572" | Venue correct (ICDE 2020, pp. 673–684, doi 10.1109/ICDE48307.2020.00064). The last author differs: Xueqi Cheng on arXiv, Jiafeng Guo at ICDE. The speed-ups differ too: 12.3×/40.0× on arXiv, 13.2×/64.8× in the ICDE abstract (snippet). |
| GraphMP | "arXiv 1707.02557" | That is the conference version. The journal version is "GraphMP: I/O-Efficient Big Graph Analytics on a Single Commodity Machine", IEEE Trans. Big Data (arXiv 1810.04334; snippet). |
| ACGraph | "doi 10.1145/3769755" | DOI correct: PACMMOD 3(6), art. 290, Dec 2025 (SIGMOD 2026). It is the same paper as survey item I.9 (arXiv 2511.07886), so it is not new. |

The **resources** all exist, but two are stale. The iDC-NEU list has 40 papers, the newest from 2023; its
disk-based section stops at 2019 (Lumos). J. Shun's page has moved to
https://jshun.csail.mit.edu/graph.shtml and says it "has not been updated since 2022". Neither is useful
for anything after 2022.

**One correction to the survey above.** Sections G.2 and I.9 put LUMOS with CLIP and ACGraph
under "asynchronous, conflicts with D14". That is too broad, in two ways:

- LUMOS and Grafu both claim **synchronous semantics**. Future-value computation computes
  iteration i+1 values early, but with the same inputs. It can be bit-identical, provided each
  vertex's sum is accumulated in a fixed order.
- For kernels whose answer is the **unique fixpoint** of a monotone update, the update order
  changes only the work done, never the output. These are min-label WCC, BFS depths, SSSP
  distances and k-core; SemiCore* (D.1) was adopted on exactly this ground. So asynchronous,
  on-demand and pruned execution is compatible with "What 'repeatable' promises" for these kernels.

The conflict is real only for PageRank-style sums whose float order would change, and for
Leiden/LPA, where the order changes the partition.

### A. Systems for graphs larger than memory

1. **Seraph** — Tsun-Yu Yang, Yizou Chen, Yuhong Liang, Ming-Chang Yang, "Seraph: Towards Scalable and Efficient Fully-external Graph Computation via On-demand Processing", USENIX FAST 2024, pp. 373–387. https://www.usenix.org/system/files/fast24-yang-tsun-yu.pdf (read). A journal extension appeared in ACM TOS 2025 (snippet).
   - What it does: a *fully external* system, with vertex values on SSD as well as edges. Instead of streaming every edge, it reads on demand only the edge lists of active vertices, relying on SSDs whose random reads are close to sequential speed. It uses three designs:
     - a hybrid format: large lists in row format, small lists in a 2D grid for locality;
     - "vertex passing": updates to vertices not in memory are deferred and delivered when their partition is loaded;
     - asynchronous "selective pre-computation" of future iterations.
   - Numbers: Twitter 1.4 B, Gsh2015 33.9 B, Eu2015 91.8 B, and RMAT with 8.6 B nodes and 112 B edges on 8-byte ids. Memory was 130 MB to 18 GB (18 GB for RMAT). Hardware: 16 threads, 2 NVMe at 6 GB/s together. Seraph is up to two orders of magnitude faster than other fully-external systems, 5.0× faster than CLIP on BFS on Gsh2015 and 37.3% faster than GridGraph on PageRank. Pre-computation adds 16.1% on average. No code found.
   - In the survey? No.
   - For us:
     - On-demand reading of active lists is what m3-01's sorted-frontier, coalesced reads and block skipping already plan (FlashGraph, GridGraph). Seraph is outside evidence that on NVMe it beats full streaming for sparse-frontier kernels.
     - Vertex passing, meaning deferred messages to state that is on disk, is the mechanism #114 would need when the O(n) state does not fit (D11 refuses today).
     - Selective pre-computation is asynchronous. It is allowed only for monotone fixpoint kernels.
   - Issues: #46 (m3-03), m3-01, #114 (m9-07).
   - Recommendation: CITE; take vertex passing as the design reference if #114 builds the fully external regime.

2. **Grafu** — Tsun-Yu Yang, Cale England, Yi Li, Bingzhe Li, Ming-Chang Yang, "Grafu: Unleashing the Full Potential of Future Value Computation for Out-of-core Synchronous Graph Processing", ASPLOS 2024 (vol. 2), pp. 467–481, doi 10.1145/3620665.3640409. https://dl.acm.org/doi/10.1145/3620665.3640409 (403); abstract via Semantic Scholar.
   - What it does: future-value computation (reusing loaded edges to compute values of later iterations early) under synchronous semantics. It uses three designs:
     - an "elevator" execution order that raises the number of vertices computed early;
     - a conditional barrier;
     - a "greedy colouring" reordering that cuts the extra memory.
   - Numbers: the abstract gives none. ACGraph's paper cites it at up to 50% less runtime and I/O than GridGraph (snippet). No code found.
   - In the survey? No (LUMOS, its predecessor, is in G.2 and I.9).
   - For us:
     - The cheap part is the *order*. A multi-pass scan that alternates direction (forward, then backward) keeps the tail of the previous pass in the page cache. A forward-only scan larger than the cache gets nothing from LRU.
     - For order-independent kernels the output is unchanged by construction: Jacobi PageRank with fixed reduction blocks, WCC rounds, SemiCore*, HyperBall.
     - The full future-value scheme could be bit-identical, but needs per-vertex accumulation in a fixed order and an identity test. It pays only when the compressed graph exceeds the budget (Friendster about 10 GB compressed at 23.2 bits/entry × 3.6 B entries, arithmetic, against an 8 GiB budget).
   - Issues: #46 (m3-03), m3-04 PageRank.
   - Recommendation: TEST the alternating-direction scan (cheap, below). LATER, future-value computation.

3. **Gem** — Chengying Huan, Zhengyi Yang, Haoshen Yang, Shaonan Ma, Rong Gu, Fang Xi, Yongchao Liu, Guihai Chen, Chen Tian, "Gem: Scalable Monotonic Graph Processing Beyond Billion-Scale on a Single Machine", PACMMOD 3(6), pp. 1–30, Dec 2025 (SIGMOD 2026), doi 10.1145/3769795. https://dl.acm.org/doi/10.1145/3769795 (403); abstract via Semantic Scholar.
   - What it does: out-of-core engine for monotonic algorithms (SSSP, BFS, reachability). A PageRank-based in-memory "sketch" yields tight bounds, used to prune at vertex and partition level.
   - Numbers: six real graphs up to ClueWeb (42.5 B edges); up to 135.40× faster than GridGraph and 12.58× faster than Wonderland out of core; up to 10.41× faster than RisGraph in memory. No code found. Beware a name clash with an unrelated SIGMOD 2026 "GEM" (multi-vector retrieval).
   - In the survey? No.
   - For us:
     - Exact pruning by bounds keeps the answer exact, so it is compatible with determinism.
     - Relevant to point-to-point and budgeted queries, where pruning partitions saves I/O.
     - Our map hierarchy is a ready-made partition with known quotient edges. The bounds could come from it rather than from a PageRank sketch (our idea, not the paper's).
   - Issues: m7-02-shortest-paths, m7-03-ooc-queries, m7-07-distance-index.
   - Recommendation: CITE; read the full text when m7-03 starts.

4. **Planar** — Yang Liu, Wenfei Fan, Shuhao Liu, Xiaoke Zhu, Jianxin Li, "A Single Machine System for Querying Big Graphs with PRAM", PVLDB 18(3):756–769, 2024 (VLDB 2025), doi 10.14778/3712221.3712240. https://www.vldb.org/pvldb/vol18/p756-liu.pdf (read).
   - What it does: runs existing PRAM algorithms over a partition into blocks, using a GRAPE-style fixpoint (PEval, then IncEval on border updates). It groups and schedules blocks adaptively at run time against a memory budget τ.
   - Numbers:
     - Friendster under a 16 GB cgroup, on an i9-7900X with a SATA SSD at 560 MB/s: WCC 54.7 s with 7.0 GB of I/O in 2 rounds; SSSP 23.1 s.
     - Against other out-of-core systems: 34.42× faster on average, up to 302.01×.
     - ClueWeb (1.7 B nodes, 7.9 B edges): WCC 465.6 s.
   - Code: https://github.com/SICS-Fundamental-Research-Center/Planar, with no licence file (GitHub API).
   - The authors *omitted CLIP "because it produces inconsistent results for multiple graph queries"*. That is outside evidence for D14's stance.
   - In the survey? No.
   - For us:
     - The block grouping depends on the budget. That is acceptable only for unique-fixpoint kernels (WCC, SSSP); for PageRank or Leiden it would break the any-budget identity.
     - Their Friendster WCC is far from ours (3.6 s streamed on raw arrays, 32 threads, NVMe), but hardware and storage differ, so this is not a like-for-like comparison.
   - Issues: #46 (m3-03), m3-02 planner (as a counter-example).
   - Recommendation: CITE (the CLIP remark in particular). Do not adopt budget-dependent scheduling for order-sensitive kernels.

5. **Kedagraph** — Bo-Cheng Liao, Shu-Chun Chou, Sun-Yuan Hsieh, "Kedagraph: memory-efficient out-of-core graph processing system with high applicability", J. Supercomputing 82(9), art. 530, 25 June 2026, CC BY, doi 10.1007/s11227-026-08638-8. https://researchoutput.ncku.edu.tw/en/publications/kedagraph-memory-efficient-out-of-core-graph-processing-system-wi/ (abstract). Springer redirected to a login page.
   - What it does: three "streamers" plus a compression scheme:
     - a grid streamer, compared with GridGraph on WCC and SpMV;
     - a primary–secondary streamer, compared with LUMOS on PageRank;
     - a vertex streamer, compared with GraphChi on community detection.
   - Numbers: none in the abstract.
   - In the survey? No.
   - For us: nothing new beyond GridGraph and LUMOS. The compression scheme was not read.
   - Recommendation: IGNORE unless the full text shows a compression result on a 2D grid (#114's 2D grid).

6. **ACGraph** — Dechuang Chen, Sibo Wang, Qintian Guo, PACMMOD 3(6) art. 290, 2025 (SIGMOD 2026), doi 10.1145/3769755. https://arxiv.org/abs/2511.07886 (abstract).
   - What it does: asynchronous, pipelined I/O and computation for SSD with limited memory.
   - In the survey? Yes, I.9.
   - For us: unchanged. It is acceptable only for monotone fixpoint kernels.
   - Recommendation: CITE ONLY.

7. **GraphMP** — Peng Sun, Yonggang Wen, Ta Nguyen Binh Duong, Xiaokui Xiao, arXiv 1707.02557 (2017). https://arxiv.org/abs/1707.02557 (abstract). The journal version is above.
   - What it does: semi-external, with vertices in RAM and edge shards on disk. It uses a "vertex-centric sliding window", skips inactive shards, and caches compressed edges in spare memory.
   - Numbers: on a billion-vertex graph, 31.6× faster than GraphChi, 54.5× than X-Stream and 23.1× than GridGraph.
   - In the survey? No.
   - For us: it is the same model as D11's semi-external regime plus D4 ("keep the compressed form resident"). This is outside evidence, nothing new to build.
   - Recommendation: CITE ONLY (#46).

8. **OMRGx** — Gurneet Kaur, Rajiv Gupta, "OMRGx: Programmable and Transparent Out-of-Core Graph Partitioning and Processing", ISMM 2023, pp. 137–149, doi 10.1145/3591195.3595268. https://doi.org/10.1145/3591195.3595268 (403); abstract via Semantic Scholar; CC BY.
   - What it does: a framework that builds and processes several partitions at once, dividing memory among them; it gives the first out-of-core MtMetis.
   - Numbers: none in the abstract.
   - In the survey? No.
   - For us: we do not partition (D11 streams).
   - Recommendation: IGNORE; possibly cite in #114 if a 2D grid needs a partitioner.

9. **GraphQ** — Kai Wang, Guoqing Xu, Zhendong Su, Yu David Liu, "GraphQ: Graph Query Processing with Abstraction Refinement — Scalable and Programmable Analytics over Very Large Graphs on a Single PC", USENIX ATC 2015, pp. 387–401. https://www.usenix.org/conference/atc15/technical-session/presentation/wang-kai (abstract).
   - What it does: answers queries by refining across a multi-level abstraction of the graph, loading only the partitions it needs; budget-aware.
   - Numbers: graphs 4–6× larger than memory, answered in seconds to minutes; 1.6–13.4× faster than an early-stopping GraphChi.
   - In the survey? No.
   - For us: the map hierarchy with its quotient graphs is such an abstraction. It is a candidate "navigation map" for budgeted out-of-core queries (`limit`/`timeout` in D21).
   - Issues: m7-03-ooc-queries, m7-07-distance-index.
   - Recommendation: CITE; re-read when m7-03 is designed.

10. **RStream** (Kai Wang, Zhiqiang Zuo, John Thorpe, Tien Quang Nguyen, Guoqing Harry Xu, OSDI 2018, pp. 763–782; https://www.usenix.org/conference/osdi18/presentation/wang, abstract) and **Kaleido** (ICDE 2020, citation above; https://arxiv.org/abs/1905.09572, abstract).
    - What they do: out-of-core graph *pattern mining* (motifs, cliques, frequent subgraphs). RStream is at least 1.7× faster than distributed miners. Kaleido is 12.3–13.2× faster than Arabesque and 40.0–64.8× faster than RStream (the two versions differ).
    - In the survey? No.
    - For us: no pattern-mining kernel is planned. For triangles (m3-08), MGT (E.1) is the better fit.
    - Recommendation: IGNORE.

### B. Memory mapping, user-space paging, SSD as memory

11. **MMap** — Zhiyuan Lin, Minsuk Kahng, Kaeser Md. Sabrin, Duen Horng Chau, Ho Lee, U Kang, "MMap: Fast Billion-Scale Graph Computation on a PC via Memory Mapping", IEEE BigData 2014, pp. 159–164. https://pmc.ncbi.nlm.nih.gov/articles/PMC4389765/ (read).
    - What it does: a binary edge list plus an 8-byte offset index, memory-mapped, with no `madvise` and no `mlock`. It relies on readahead and LRU keeping high-degree lists resident.
    - Numbers: 16 GB RAM, SSD. 1.11–2.15× faster than TurboGraph and 1.34–16.4× faster than GraphChi (9.5× on PageRank on Twitter, 1.47 B edges). On YahooWeb (6.6 B edges) only "slightly better", with the node vectors themselves memory-mapped.
    - In the survey? No.
    - For us: it shows mmap works for *scans* and for a hot set that fits. That is our "mmap warm" regime, already measured (D11). It says nothing about random access under pressure, which is the tracer's case. M-Flash (ECML-PKDD 2016) reports MMap thrashing when memory is very limited (search snippet only, not read).
    - Recommendation: CITE ONLY.

12. **UMap** — Ivy B. Peng, Marty McFadden, Eric Green, Keita Iwabuchi, Kai Wu, Dong Li, Roger Pearce, Maya Gokhale, "UMap: Enabling Application-driven Optimizations for Page Management", MCHPC@SC 2019, pp. 71–78, doi 10.1109/MCHPC49590.2019.00017; arXiv 1910.07566. https://arxiv.org/abs/1910.07566 (read).
    - What it does: user-space paging through `userfaultfd`, with an application-chosen page size, readahead, buffer size and eviction.
    - Numbers:
      - 1.25–2.5× faster than mmap on average.
      - Sorting 500 GiB: UMap beats mmap only above 64 KiB pages, and is 2.5× faster at 8 MiB.
      - BFS on a CSR from SSD: **mmap's best case was readahead off**; UMap at 512 KiB pages was 1.8× faster than that.
    - Code: llnl/umap, C++, LGPL-2.1 (GitHub API).
    - In the survey? No.
    - For us:
      - A user-space pager with a fixed buffer would be accountable, unlike the page cache. But it is a second pager to maintain, has no wasm32 or macOS path, and under LGPL it cannot be vendored into an Apache/MIT core (D23).
      - Its lesson is cheaper to take: for random access, mmap's default read-around is the wrong policy, and a large, application-chosen read unit is the right one. Explicit `pread`/`io_uring` reads of coalesced Elias–Fano byte ranges into an accounted buffer give both without `userfaultfd`.
    - Issues: m2-02-io-layer (`advise()`), #67 (m5-01), m7-03.
    - Recommendation: CITE; do not adopt `userfaultfd`.

13. **FastMap** — Anastasios Papagiannis, Giorgos Xanthakis, Giorgos Saloustros, Manolis Marazakis, Angelos Bilas, "Optimizing Memory-mapped I/O for Fast Storage Devices", USENIX ATC 2020, pp. 813–827. https://www.usenix.org/conference/atc20/presentation/papagiannis (abstract).
    - What it does: a redesigned Linux mmap path (a kernel module).
    - Numbers: stock mmap "does not scale beyond 8 threads" on 32 cores. FastMap reaches 80 cores, up to 11.8× more IOPS on null_blk and 5.27× more throughput on Optane.
    - Code: CARV-ICS-FORTH/FastMap, licence not asserted.
    - In the survey? No.
    - For us: it needs a custom kernel module, so it is out of the question for a library. The lesson is that fault-driven I/O at 32 threads is lock-bound.
      - Not verified here: Linux has since moved page faults to per-VMA locks (from 6.4 for anonymous memory, later for file mappings), which postdates the paper; this machine runs 6.12.
      - A random-access stage under memory pressure should therefore issue explicit reads rather than fault on a mapping.
    - Recommendation: CITE ONLY.

14. **AIFM** (Zhenyuan Ruan, Malte Schwarzkopf, Marcos K. Aguilera, Adam Belay, OSDI 2020, pp. 315–332; https://www.usenix.org/conference/osdi20/presentation/ruan, abstract) and **SSDAlloc** (Anirudh Badam, Vivek S. Pai, NSDI 2011; https://www.usenix.org/conference/nsdi11/ssdalloc-hybrid-ssdram-memory-management-made-easy, page opened, abstract via search).
    - What they do: object-granular far memory (AIFM, remote memory, up to 61× faster than Fastswap) and SSD used as an extension of RAM (SSDAlloc, 90% of the SSD's raw performance).
    - In the survey? No.
    - For us: both make memory beyond RAM look like RAM. That conflicts with the explicit accountant (D11, m2-04, m2-19), and AIFM's target, network memory, is out of scope.
    - Recommendation: IGNORE.

### C. External-memory libraries

15. **TPIE** — Lars Arge, Octavian Procopiuc, Jeffrey S. Vitter, "Implementing I/O-efficient Data Structures Using TPIE", ESA 2002, LNCS 2461, pp. 88–100, doi 10.1007/3-540-45749-6_12. Lars Arge, Mathias Rav, Svend C. Svendsen, Jakob Truelsen, "External Memory Pipelining Made Easy With TPIE", IEEE BigData 2017, pp. 319–324, doi 10.1109/BIGDATA.2017.8257940. https://arxiv.org/abs/1710.10091 (abstract).
    - What it does: a C++ external-memory library. Its pipelining framework decides which stages to chain and assigns each stage its share of main memory automatically.
    - Code: thomasmoelhave/tpie, licence not asserted by GitHub (not verified).
    - In the survey? No.
    - For us: automatic division of one memory budget among the stages of a pipeline is the same problem as the planner and accountant face in ingest: parse, then dedupe, then CSR build, then encode.
    - Issues: m3-02 planner, m2-04 accountant.
    - Recommendation: CITE; read the 2017 paper's memory-assignment rule before m3-02 fixes its own.

16. **STXXL** — Roman Dementiev, Lutz Kettner, Peter Sanders, ESA 2005, LNCS 3669, pp. 640–651, doi 10.1007/11561071_57; Software: Practice and Experience 38(6):589–637, 2008 (online 2007), doi 10.1002/spe.844 (Crossref).
    - What it does: an external-memory STL. Its pipelining "can save more than half of the I/Os".
    - Licence: BSL-1.0 (GitHub API).
    - In the survey? Named only in the licence list.
    - For us: a reference for sort-based aggregation and spill (m1-02, m4-03). Nothing to vendor (D1).
    - Recommendation: CITE ONLY.

### D. Resources

17. **LDBC Graphalytics** — https://ldbcouncil.org/benchmarks/graphalytics/ (opened), specification v1.0.5, arXiv 2011.15028 v6 (2023, read; CC BY 4.0), datasets page https://ldbcouncil.org/benchmarks/graphalytics/datasets/ (opened), fair-use policy https://ldbcouncil.org/benchmarks/fair-use-policies/ (opened). The competition-rules URL returned 404.
    - **Algorithms (six, all deterministic by specification):**
      - BFS depth: exact match.
      - PageRank: a fixed number of iterations, **f64 required**, sink mass spread uniformly; epsilon match, |r − s| ≤ 10⁻⁴·|r|.
      - WCC on the undirected interpretation: match up to relabelling.
      - CDLP: synchronous label propagation, initial label = **vertex id**, ties → **smallest label**; exact match.
      - LCC, directed definition included: epsilon match.
      - Weighted SSSP: epsilon match.
    - **Reference outputs** are provided with the datasets (some were corrected in March 2023).
    - **Datasets:** about 350 GB compressed, 1.5 TB raw; Parquet copies since February 2026. They include:
      - real graphs: com-friendster, twitter_mpi, cit-Patents, wiki-Talk, dota-league, kgs;
      - synthetic Graph500 scales 22–30, the largest graph500-30 with 447 M nodes and 17 B edges (3XL);
      - LDBC Datagen up to sf10k-fb, with 100 M nodes and 9 B edges;
      - tiny test-* graphs per algorithm.
      Scale classes are ⌊10·log₁₀(n+m)⌋/10, from 2XS to 3XL.
    - **Run rules:** each algorithm is run 3 times. Timeouts are 15 min for S, 30 min for M, 1 h for L, 2 h for XL and 3 h for 2XL+. Reported: processing time, makespan, SLA breaches and EVPS (edges plus vertices per second).
    - **Licences:** driver ldbc/ldbc_graphalytics Apache-2.0, GraphBLAS reference platform Apache-2.0 (GitHub API). The datasets' licence is **not stated** on the page (not verified); the real graphs come from SNAP, KONECT and others under their own terms.
    - **Reporting:** "LDBC Benchmark Result" is a registered trademark use reserved for **audited** results. Unaudited numbers must not use the name and should say "These are not LDBC Benchmark Results" and how they differ.
    - In the survey? No; the repository mentions LDBC only for SNB in m9-03.
    - For us, three uses:
      - **A big-graph oracle.** m0-05 stops at 10⁵ edges. LDBC gives reference outputs for BFS, WCC, PageRank, LCC, SSSP and CDLP on billion-edge graphs, including Friendster, which we already use. These are the tests m3-11 and the "same result for any budget" promise lack at scale.
      - **A recognised workload for #100,** run with its rules (3 repetitions, timeouts) and published with the disclaimer. Graph500-30 (17 B edges) is a public bigger-than-RAM case for the 61 GB machine.
      - **Semantic checks:**
        - Our PageRank S1 formula with uniform teleport equals LDBC's in f64; the f32 default does not qualify.
        - m4-09's "tie-break by label hash" does **not** match CDLP's smallest-id rule.
        - Since CDLP labels and ties are defined on the *input's* ids, a seeded relabelling at ingest (#146) must compare original ids, or CDLP will fail validation.
    - Issues: #100 (m8-03), m0-05-oracle, m3-11-determinism-ci, m3-04, m3-05, m3-06, m3-08, m4-09, m7-02, #146.
    - Recommendation: **ADOPT** the datasets and reference outputs as oracle and benchmark workload; add an `ldbc` conformance mode or test profile; never call our numbers LDBC results.

18. **Web Data Commons hyperlink graph** — https://webdatacommons.org/hyperlinkgraph/ and https://webdatacommons.org/hyperlinkgraph/2012-08/download.html (opened).
    - **Contents:**
      - 2012 crawl: page graph 3,563 M nodes and 128,736 M arcs; host graph 101 M nodes and 2,043 M arcs; pay-level-domain graph 43 M nodes and 623 M arcs.
      - 2014 crawl: page graph 1,727 M nodes and 64,422 M arcs.
      - The 2012 page graph comes as 45 GB of index files plus 331 GB of arc files, or as a WebGraph BV file of 52 GB plus 4 GB of offsets (about 3.3 bits/arc, arithmetic).
    - **Terms:** the data falls under Common Crawl's terms of use; the extraction code is Apache-2.0.
    - **Against 2^32:** **no version exceeds 2^32 nodes.** 3.56 B is 83% of 4.29 B. The arc count far exceeds 2^32, which u64 offsets already cover (D3). So it is a near-limit u32 test, not a test of #114's "> 2^32 nodes" acceptance.
    - At 3.56 B nodes a single 4-byte state array is 14.3 GB (arithmetic), so Leiden's O(n) state alone exceeds 16 GiB. This is where the fully external regime would first be needed.
    - At our ~16–23 bits/entry it would be roughly 260–370 GB compressed (estimate, not measured; web graphs not validated for D4).
    - In the survey? No.
    - For us: the **host graph** (2 B arcs, real web topology) is a cheap web-family graph for #24, beside uk-2005. The page graph is a server-scale target for #114's "≥ 10 B-edge public web graph" criterion.
    - Recommendation: ADOPT the host graph for #24. The page graph is for #114 only, which will still need a synthetic graph above 2^32 nodes (Seraph used RMAT with 8.6 B nodes and 8-byte ids) or Software Heritage.

19. **iDC-NEU Graph-Computing-Papers** (https://github.com/iDC-NEU/Graph-Computing-Papers, opened; no licence) and **J. Shun's "Papers on Graph Analytics"** (https://jshun.csail.mit.edu/graph.shtml, opened; not updated since 2022).
    - Reading lists, both older than everything in this addendum.
    - Recommendation: Shun's page is a good index of classics (external-memory and compression sections). Neither adds anything after 2022. CITE ONLY.

### E. The mmap question: what would fix the layout's 733 GB under 16 GiB

The tracer's layout (FINDINGS § M1-01, row 19) read 733 GB for a 15.5 GB CSR, about 47× (arithmetic), while about 12 GiB remained for page cache. The kernel behaviour below is from the Linux sources and documentation, not measured here:

- With no `madvise`, a fault on a file mapping triggers *read-around* of the readahead window, `read_ahead_kb`, typically 128 KiB.
- Friendster's average list is about 220 B (55 entries × 4 B). A random touch therefore brings in roughly 500–600× its useful bytes, and the cache thrashes.

What the literature supports:

1. `MADV_RANDOM` on the mapping for random-access stages. This turns read-around off, which was mmap's best setting for BFS in UMap. It is one line, keeps results identical, and is the cheapest test, but it treats the symptom.
2. Fix the access pattern instead of the pager. This is the real fix, and it is deterministic by construction:
   - The layout needs, per subtree, the edges between siblings and inside leaf communities. One *sequential* pass over the store can bucket edges by finest community into spilled, budgeted buckets, the same machinery as #16's sequential aggregation and Graphyti's re-stream through `community[]` (survey A.3).
   - Each subtree is then laid out from its own bucket.
   - Equivalently, renumber by hierarchy (m1-09) so that a subtree's lists are contiguous in the store.
   - The tile builder (#69) needs the same pass; its issue already asks for "one adjacency pass".
3. Explicit reads instead of faults for what stays random. Use `pread` or `io_uring` of coalesced Elias–Fano ranges into an accounted buffer, with UMap's large application-chosen unit (64 KiB–1 MiB). FastMap's result argues against fault-driven I/O at 32 threads.
4. Not recommended:
   - `userfaultfd` / UMap: LGPL, Linux-only, a second pager.
   - FastMap: a kernel module.
   - SSDAlloc / AIFM: transparent memory beyond RAM, which conflicts with the accountant.

### F. Ranked recommendations from this list

1. **LDBC Graphalytics as oracle and workload** (#100, m0-05, m3-11, the kernels).
   - Cheap test: download the test-* graphs and example graphs plus graph500-22 (64 M edges). Validate the lab PageRank (f64), WCC and BFS against the reference outputs with LDBC's three match rules, and record each mismatch as a semantic difference.
2. **Layout reads the graph sequentially** (#67; also #69, m1-09).
   - Cheap test: on orkut, with a `MemoryMax` at about a quarter of the CSR size, run the lab layout three ways and record time and `read_bytes`:
     - mmapped, as it is today;
     - with `MADV_RANDOM`;
     - on an input renumbered by hierarchy.
   - This takes minutes, and one patch adds the `madvise`.
3. **`advise()` policy in the I/O layer** (m2-02, m7-03): `MADV_RANDOM` (or explicit coalesced reads) for random paths, `MADV_SEQUENTIAL`/`WILLNEED` for scans.
   - Cheap test: the cold point-query benchmark of FINDINGS § B1 with and without `MADV_RANDOM`, bytes read per query.
4. **Alternating-direction scans for order-independent multi-pass kernels** (#46, m3-04).
   - Cheap test: Friendster PageRank on the compressed store (about 10 GB) under `MemoryMax` = 8 GiB, forward-only against alternating, 10 iterations. Record `read_bytes` per iteration and check the output hash is identical.
   - Also on orkut scaled down: a budget at half its compressed size.
5. **Order-free execution for monotone kernels** (#46, m3-05, m3-06, m3-07, m7-02): on-demand reading of active lists (Seraph), bound-based pruning (Gem), and asynchronous rounds where they cut I/O. The output is the unique fixpoint, so it is the same for any order.
   - Cheap test: Friendster WCC out of core with per-round active fractions logged. Compare bytes read when decoding only active nodes' lists (Elias–Fano offsets) with the full scan's 3.6 s. Assert identical labels.

### G. What this list suggests about decisions

- **The survey's blanket "asynchronous → NO" (G.2, I.9) is too strong**, as argued at the top of this addendum. ADR 0014 and "What 'repeatable' promises" stay as they are. Only the survey's recommendation narrows: it applies to order-sensitive kernels (PageRank sums, Leiden, LPA), not to monotone fixpoints. LUMOS and Grafu are synchronous.
- **m4-09's hash tie-break** is a defensible choice, but it makes our LPA differ from the only standard deterministic LPA (LDBC CDLP: smallest original id). Decide this consciously; offer an `ldbc` variant if not.
- **#146 (relabel at ingest)** must keep id-dependent semantics on original ids: CDLP ties, min-label WCC outputs, BFS from a given source. Otherwise LDBC validation fails.
- **#114's acceptance** ("> 2^32 nodes") cannot be met by Web Data Commons (3.56 B nodes); the issue should name the dataset. LDBC's graph500-30 (447 M nodes) does not help either.
- **D11's "mmap under pressure" regime** is supported only for scans. Under pressure the tracer's random-access layout re-read 47×, and UMap found mmap's best random-access setting is readahead off. The planner should not choose plain mmap for a random-access stage under a budget smaller than its working set: it should use explicit reads or a sequential restructuring. This sharpens D11 rather than reversing it, and #67 and m2-02 are where it lands.
- **PageRank's f32 default** (m3-04) is fine for the product, but LDBC requires f64. Published comparisons (#100) must use f64.
