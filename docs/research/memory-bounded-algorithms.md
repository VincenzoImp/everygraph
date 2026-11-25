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
   - Licence: not verified.
   - Lesson: deterministic push reductions.
   - Recommendation: PROTOTYPE the idea.

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
- Licences not verified: Graphyti, Vite, puzzlef's leiden-lowmem, MGT code, STXXL, Blaze, Graphene, Aspen, BatchLayout, DRGraph, SNAP-tFDP, t-FDP. OGDF returned NOASSERTION.
- Unknown whether KaMinPar has a deterministic mode.
- Seen only as search snippets: BP reordering details, BuffCut, HeiStream, KADABRA's speed-up figures, Ajwani et al.'s BFS timings.
- Bounds not read: the exact I/O bounds of Abello et al., 1PSCC and Mehlhorn–Meyer; and the contents of Contract & Expand and Wan & Wang's SCC paper (pages blocked).
- No code found for SemiCore* or 1PSCC. EP-BFS's baselines and absolute times not verified.
- Venue of PowerPush, and HyperBall's pass count on Friendster, not verified.
- No figures beyond their abstracts for GridGraph, Graphene, LUMOS, GraphChi or X-Stream.
- NetworKit PLM's "3.3 B edges" figure not confirmed.
- The ACM out-of-core graph-processing survey (DOI 10.1145/3838595) returned 403 and was not read.
