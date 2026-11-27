# State of the art — maps: layout, level of detail and tiles for very large graphs

Status: research note, 2026-11-25. Scope: area "maps" of the SOTA review (decisions D16,
D17, D18, D24/D25; issues #19 `m1-05`, #20 `m1-06`, #21 `m1-07`, #26 `m1-12`, #63 `m4-07`,
#67 `m5-01`, #68 `m5-02`, #69 `m5-03`, #71 `m5-05`, `m5-07`, `m9-05`). Nothing in the
repository was built or run; no graph was loaded. Numbers quoted from papers carry the
paper's machine, dataset and threads and are **not comparable to ours as they are**.
Our own numbers are from `lab/e-map/results/` and FINDINGS § E and § M1-01.

## 1. Scope and reading log

PDFs are in `/home/tux/graphlab/papers/maps/`, text in `…/maps/txt/`. Read level:
**full** = method and evaluation read by me; **full (delegated)** = read in full by a
sub-reader with a fixed question list (two readers; six layout papers by a further
helper), key numbers spot-checked by me in the text (SNAP-tFDP Friendster row, DRGraph
com-Orkut row, BatchLayout determinism sentence, FA2 Noack metric); **partial** = method
or results sections only; **abstract** = abstract/results from the paper or its landing
page; **not accessible** = no open copy obtained.

| Work | Venue, year | Link | Read | PDF |
|---|---|---|---|---|
| GMap: drawing graphs as maps (Gansner, Hu, Kobourov) | PacificVis 2010 | arxiv.org/abs/0907.2585 | full | 2010-gansner-gmap.pdf |
| MapSets (Efrat, Hu, Kobourov, Pupyrev) | GD 2014 / JGAA 2015 | doi.org/10.7155/jgaa.00364 | abstract | 2015-efrat-mapsets.pdf |
| GraphMaps (Nachmanson et al.) | GD 2015 | arxiv.org/abs/1506.06745 | full | 2015-nachmanson-graphmaps.pdf |
| New approach to GraphMaps (Mondal, Nachmanson) | IVAPP 2018 | arxiv.org/abs/1705.05479 | abstract | 2018-mondal-graphmaps-new-approach.pdf |
| MSAGLJS tile pyramids + sleeve routing (Nachmanson, Chen) | arXiv 2026 | arxiv.org/abs/2605.17498 | full (§1–4.2, Table 1) | 2026-msagljs-tiles.pdf |
| Multi-scale community visualization of massive graphs (Langevin et al., Uncharted) | IEEE Graphs at Scale 2015 | uncharted.software/assets/multi-scale-community-visualization.pdf | full | 2015-langevin-multiscale-community-tiles.pdf |
| ASK-GraphView (Abello, van Ham, Krishnan) | TVCG 2006 | doi.org/10.1109/TVCG.2006.120 | partial (architecture, clustering) | 2006-abello-ask-graphview.pdf |
| graphVizdb (Bikakis et al.) | ICDE 2016 | arxiv.org/abs/1602.06401 | abstract | 2016-bikakis-graphvizdb.pdf |
| Hierarchical aggregation framework (Bikakis et al.) | arXiv 2015/2017 | arxiv.org/abs/1511.04750 | downloaded, not read | 2017-bikakis-hierarchical-aggregation-framework.pdf |
| Graph Cities (Abello et al.) | BigVis@EDBT 2021 | ceur-ws.org/Vol-2841/BigVis_5.pdf | abstract | 2021-abello-graph-cities.pdf |
| BigGraphVis (streaming + GPU) | arXiv 2021 | arxiv.org/abs/2108.00529 | abstract | 2021-biggraphvis.pdf |
| ZMLT zoomable multilevel tree (De Luca et al.) | arXiv 2019 | arxiv.org/abs/1906.05996 | full (delegated) | 2019-zmlt-zoomable-multilevel-tree.pdf |
| Interactive LOD rendering of large graphs (Zinsmaier et al.) | TVCG 2012 | graphics.uni-konstanz.de (author PDF) | abstract | 2012-zinsmaier-lod-rendering.pdf |
| Grouse (Archambault, Munzner, Auber) | EuroVis 2007 | cs.ubc.ca (author PDF) | abstract | 2007-archambault-grouse.pdf |
| Drawing clustered graphs as topographic maps (Gronemann, Jünger) | GD 2012 | kups.ub.uni-koeln.de/55029 | not accessible (served HTML); abstract via landing page | — |
| Rapid graph layout using space-filling curves (Muelder, Ma) | TVCG 2008 | vis.cs.ucdavis.edu (author PDF) | full | 2008-muelder-sfc-layout.pdf |
| Treemap-based rapid layout of large graphs (Muelder, Ma) | PacificVis 2008 | vis.cs.ucdavis.edu (author PDF) | downloaded, not read | 2008-muelder-treemap-layout.pdf |
| GosperMap (Auber et al.) | TVCG 2013 | doi.org/10.1109/TVCG.2013.91 | partial (method) | 2013-auber-gospermap.pdf |
| Circle packing for large hierarchies (Wang et al.) | CHI 2006 | doi.org/10.1145/1124772.1124851 | not accessible (ACM); method via citing papers | — |
| Bubble Treemaps (Görtler et al.) | TVCG 2018 (VIS 2017) | doi.org/10.1109/TVCG.2017.2743959 | abstract + intro | 2018-goertler-bubble-treemaps.pdf |
| Computing Voronoi treemaps (Nocaj, Brandes) | EuroVis 2012 | kops.uni-konstanz.de (nb-cvt-12.pdf) | partial (method) | 2012-nocaj-voronoi-treemaps.pdf |
| Neighbourhood-preserving Voronoi treemaps (Paetzold et al.) | TVCG 2025 | arxiv.org/abs/2508.03445 | partial (method) | 2025-neighborhood-preserving-voronoi-treemaps.pdf |
| NCP: neighbourhood-preserving non-uniform circle packing (Li et al.) | CVM, arXiv 2026 | arxiv.org/abs/2602.00668 | full | 2026-ncp-neighborhood-preserving-circle-packing.pdf |
| VMap (rectangular map-like vertex-centric exploration) | arXiv 2023 | arxiv.org/abs/2306.00120 | downloaded, not read | 2023-vmap.pdf |
| Overview + detail layout for compound graphs | arXiv 2024 | arxiv.org/abs/2408.04045 | downloaded, not read | 2024-overview-detail-compound-graphs.pdf |
| ForceAtlas2 (Jacomy et al.) | PLoS ONE 2014 | doi.org/10.1371/journal.pone.0098679 | full (delegated) | 2014-jacomy-forceatlas2.pdf |
| OpenOrd (Martin et al.) | SPIE VDA 2011 | doi.org/10.1117/12.871402 | not accessible (paywall; slides URL dead); abstract | — |
| Efficient high-quality force-directed drawing, basis of sfdp (Hu) | Mathematica J. 2005/06 | content.wolfram.com/sites/19/issue/v10i1.pdf | full (delegated) | 2005-hu-sfdp-mathematica-issue.pdf (txt: 2005-hu-sfdp.txt) |
| Large-graph layout algorithms at work: FM³, GRIP, HDE, ACE… (Hachul, Jünger) | JGAA 2007 | doi.org/10.7155/jgaa.00150 | full (delegated) | 2007-hachul-large-graph-layout-at-work.pdf |
| Pivot MDS (Brandes, Pich) | GD 2006 | uni-konstanz.de (BrPi06.pdf) | full (delegated) | 2006-brandes-pivot-mds.pdf |
| Sparse stress model (Ortmann, Klimenta, Brandes) | GD 2016 / JGAA 2017 | arxiv.org/abs/1608.08909 | full (delegated) | 2016-ortmann-sparse-stress.pdf |
| Graph drawing by SGD (Zheng, Pawar, Goodman) | TVCG 2018 | arxiv.org/abs/1710.04626 | full (delegated) | 2018-zheng-sgd-graph-drawing.pdf |
| (SGD)² multicriteria (Ahmed et al.) | arXiv 2021/22 | arxiv.org/abs/2112.01571 | full (delegated) | 2022-ahmed-sgd2-multicriteria.pdf |
| tsNET (Kruiger et al.) | EuroVis 2017 | doi.org/10.1111/cgf.13187 | full (delegated) | 2017-kruiger-tsnet.pdf |
| BH/FIt/L-tsNET (Meidiana et al.) | arXiv 2025 | arxiv.org/abs/2509.19785 | full (delegated) | 2025-fast-tsnet.pdf |
| LargeVis (Tang et al.) | WWW 2016 | arxiv.org/abs/1602.00370 | full (delegated; k-NN internals skipped) | 2016-tang-largevis.pdf |
| DRGraph (Zhu et al.) | TVCG 2020 | arxiv.org/abs/2008.07799 | full (delegated) | 2020-zhu-drgraph.pdf |
| t-FDP (Zhong et al.) | arXiv 2023 | arxiv.org/abs/2303.03964 | full (delegated) | 2023-zhong-tfdp.pdf |
| SNAP-tFDP (Chen et al.) | arXiv 2026 (VIS 2026) | arxiv.org/abs/2608.01907 | full (delegated) | 2026-snap-tfdp.pdf |
| BatchLayout (Rahman et al.) | PacificVis 2020 | arxiv.org/abs/2002.08233 | full (delegated) | 2020-rahman-batchlayout.pdf |
| GPU ForceAtlas2 (Brinkmann, Rietveld, Takes) | ICPP 2017 | liacs.leidenuniv.nl/~takesfw/pdf/exploiting-gpus-fast.pdf | abstract | 2017-brinkmann-gpu-forceatlas2.pdf |
| Size should not matter: stress (Ahmed et al.) | arXiv 2024/26 | arxiv.org/abs/2408.04688 | abstract | 2024-wang-size-should-not-matter-stress.pdf |
| "Normalized stress" is not normalized (Smelser et al.) | arXiv 2024/25 | arxiv.org/abs/2408.07724 | abstract | 2024-smelser-normalized-stress.pdf |
| Hierarchical edge bundles (Holten) | TVCG 2006 | doi.org/10.1109/TVCG.2006.147 | full (delegated) | 2006-holten-hierarchical-edge-bundles.pdf |
| Edge-path bundling (Wallinger et al.) | TVCG 2021 | arxiv.org/abs/2108.05467 | full (delegated) | 2021-wallinger-edge-path-bundling.pdf |
| Fast and faithful bundling by spectral sparsification | arXiv 2026 | arxiv.org/abs/2604.26994 | full (delegated) | 2026-spectral-sparsification-bundling.pdf |
| Edge and trail bundling STAR (Lhuillier, Hurter, Telea) | EuroVis 2017 | doi.org/10.1111/cgf.13213 | not accessible; abstract | — |
| Hierarchical aggregation for infovis (Elmqvist, Fekete) | TVCG 2010 | cs.au.dk/~elm/pdf/hieragg.pdf | full (delegated) | 2010-elmqvist-hierarchical-aggregation.pdf |
| Node, node-link, node-link-group (Saket et al.) | TVCG 2014 | arxiv.org/abs/1404.1911 | full (delegated) | 2014-saket-node-link-group-eval.pdf |
| Group information on node-link diagrams (Jianu et al.) | TVCG 2014 | yifanhu.net/PUB/paper_TVCG2014_groupinfo.pdf | full (delegated) | 2014-jianu-group-info-eval.pdf |
| Scalability from a cognitive-load perspective (Yoghourdjian et al.) | TVCG 2021 | arxiv.org/abs/2008.07944 | full (delegated) | 2021-yoghourdjian-cognitive-load-scalability.pdf |
| Limits of complexity: survey of 152 studies (Yoghourdjian et al.) | arXiv 2018 | arxiv.org/abs/1809.00270 | full (delegated) | 2018-yoghourdjian-limits-complexity-survey.pdf |
| Node-link vs matrix, revisited (Okoe et al.) | GD 2017 | arxiv.org/abs/1709.00293 | full (delegated) | 2017-okoe-nodelink-vs-matrix-revisited.pdf |
| Node-link vs matrix (Ghoniem, Fekete, Castagliola) | InfoVis 2004 | doi.org/10.1109/INFVIS.2004.1 | not accessible (HAL challenge); cited via Okoe | — |
| Visualization tasks for unlabelled graphs (Oddo et al.) | arXiv 2025 | arxiv.org/abs/2504.14115 | full (delegated, §3–4 skimmed) | 2025-visualization-tasks-unlabelled-graphs.pdf |
| Embedding Atlas (Ren et al., Apple) | arXiv 2025 | arxiv.org/abs/2505.06386 | abstract + benchmark figure | 2025-ren-embedding-atlas.pdf |
| deepscatter / Nomic Atlas (quadtree Arrow tiles) | software | github.com/nomic-ai/deepscatter | README only | — |
| map-of-github (Kashcha): Leiden + per-cluster force layout + MapLibre vector tiles | software | github.com/anvaka/map-of-github | README only | — |
| cosmos.gl / Cosmograph; cuGraph FA2; Graphistry; sigma.js; Gephi Lite | software / blogs | cosmos.gl; medium.com/rapids-ai | vendor pages only | — |
| Cornac (Perrot, Auber) | IEEE TBD 2020 | doi via semanticscholar | not accessible; one-line description in MSAGLJS | — |

## 2. The state of the art, as a short map

**Global layout of the whole graph** (no hierarchy given):

| Family | Best known | Largest run in the paper (paper's machine) | Quality evidence | Deterministic? memory |
|---|---|---|---|---|
| Multilevel force-directed | sfdp (Hu 2005), FM³ | sfdp: 224,843 nodes, 290.6 s, Pentium 4 3 GHz; FM³: 143,437 nodes, 355.9 s, Pentium 4 2.8 GHz (Hachul–Jünger Table 2) | FM³ best stress among older methods on large graphs, conceded by DRGraph (com-LJ stress FM³ .147 vs DRGraph .2018) | Hu: octree depth chosen by operation count "so that the layout is reproducible"; otherwise seeded; in RAM |
| Continuous FD for users | ForceAtlas2 | 23,133 nodes; "not adapted to networks bigger than 100,000 nodes" | Noack normalized edge length (= our ratio, with means) | paper says "not deterministic" |
| t-SNE/UMAP-like with negative sampling | DRGraph, t-FDP, SNAP-tFDP, LargeVis | **SNAP-tFDP: com-friendster 65.6 M nodes / 1.8 B edges in 4287.3 s, 16 threads, 29.48 GB peak, Threadripper 3990X, 220 GB RAM**; DRGraph com-Orkut 1994 s 1 thread, 18.4 GB, i7-6700; t-FDP com-LJ 30 s on RTX 2080 vs DRGraph 2958 s, sfdp 5745 s | NP r=2: on orkut/LJ ≤ 0.04 for **every** method (t-FDP, SNAP-tFDP); com-orkut stress t-FDP .161, sfdp .255, DRGraph .290 | HOGWILD races + sampling; SNAP-tFDP parallel within ±5% of serial; full edge list + positions in RAM |
| Distance/stress based | stress majorization, sparse stress, SGD stress, Pivot MDS, HDE | sparse stress 20,657 nodes, 10.44 s vs 1016.92 s full; Pivot MDS 143,437 nodes 11.06 s (50 pivots, Pentium-M); HDE fe_ocean 3.4 s | sparse stress within ~1–5 % of full stress; SGD matches/beats majorization on 242 of 243 graphs ≤ 1000 nodes | Pivot MDS / HDE deterministic given pivots; BFS passes streamable; Θ(kn) |
| Batch-synchronous FD | BatchLayout | Flan_1565, 1.56 M nodes, 114 M edges, 224.3 s/500 it., 47 threads, 2× Xeon 8160 | stress best on most graphs ≤ 22k nodes | **"same layout irrespective of number of threads"** (§4.5); in RAM |
| GPU FD | GPU FA2 (Brinkmann), cuGraph FA2, cosmos.gl | 4 M nodes / 120 M edges in 14 min vs 9 h CPU (GTX Titan X, 500 it.); cuGraph claims 3–6× over that (blog) | — | atomics: not deterministic |

**Hierarchy-first ("clustered graph as map")**: GMap (embed with sfdp → modularity clusters →
Voronoi of points + random points, cells merged; O(|V| log |V|), 440k vertices in 4 min, 1
thread) gives map shapes over a global layout; it does not nest or zoom. Uncharted (2015)
is the closest pipeline to ours: distributed Louvain hierarchy → recursive force-directed
layout of communities as circles sized by count, each level inside its parent → data-tile
pyramid (Avro in HBase) → web map; largest shown 2.37 M nodes / 9.9 M links; **no quality,
cost or memory numbers**. ASK-GraphView (2006) built a hierarchy with **maximum fan-out
IL = 64 chosen empirically**, recursive re-clustering of oversized clusters and
memory/disk-resident antichains, up to 16 M edges. Muelder–Ma (2008) lay out a cluster
hierarchy by DFS order along a Hilbert/Peano/Gosper curve: Θ(|V|) layout, Θ(|V|) extra
memory, 1.07 M nodes / 2.7 M edges in 0.8 s for the curve step after 20.5 s clustering, one
core of a 2.66 GHz Xeon; quality shown only visually. GosperMap and Voronoi treemaps
(Nocaj–Brandes: power diagrams, O(n log n) per Lloyd iteration, resolution-independent)
fill the plane with containment; neighbourhood-preserving variants (Paetzold et al. 2025;
NCP 2026) initialise children from a projection of their similarity and keep the resulting
adjacency during optimisation. NCP's measured results (≤ 1083 circles, one level): NP₁ 0.360
vs 0.259 for similarity-aware front-chain packing, envelope compactness 0.874 vs 0.851, in
0.2–2.0 s.

**Zoomable / tiled viewing**: GraphMaps (2015) and MSAGLJS (2026) build tile pyramids
whose node set per level is an importance (PageRank) prefix under a per-tile quota, with
routed and bundled edges; GraphMaps took < 6 h for 38,395 nodes (Xeon E5-2690), MSAGLJS's
largest graph is 32,768 nodes / 236,978 edges and the densest tile holds 6–8× its nominal
capacity of 500 elements (Table 1). graphVizdb stores per-level layouts in R-trees behind a
DB. Graph Cities handled Friendster (1.8 B edges) in "a few minutes" but by edge-core
decomposition into 3D buildings, not a node-link map. Outside graph drawing,
deepscatter (Arrow quadtree tiles, static hosting) and Embedding Atlas (WebGPU, 60 fps up
to 4 M points on an M1 Pro) are the practical bar for point rendering; anvaka's
map-of-github (Leiden clusters, per-cluster force layout, tippecanoe vector tiles,
MapLibre, ~690k repositories) is the closest public practice to our product at small scale.

**Edges and clutter**: node-link path reading fails beyond 100 / 75 / 50 nodes at density
2 / 4 / 6 (> 50 % wrong or unsure, Yoghourdjian 2021 §4.1); 70 % of user studies with
> 200 nodes show only part of the graph (survey §IX-A); contiguous group regions help group
tasks at no cost to other tasks (Saket 2014: group accuracy 94.83 %, 10.8 s vs 13.4 s
node-link); every bundling method raises ambiguity over straight lines and some invent
structure on noise (Wallinger 2021); bundling evidence is informal (Holten) or metric-only.

## 3. Decision by decision

| Ours | Best known work | Behind / level / ahead | Evidence |
|---|---|---|---|
| **D16** two outputs; map hierarchy fan-out-capped (cap 64), levels compacted | ASK-GraphView: max fan-out IL = 64, oversized clusters re-clustered recursively, peeling + biconnected fallback for dense parts; Uncharted: plain Louvain levels | **Level** on the idea, **ahead** on scale and on typing artefact regions; ASK-GraphView is 20-year-old precedent for #19's "top-down recursive sub-clustering" candidate | ASK-GraphView §4 (IL = 64 "reasonable trade-off"); nobody reports modularity cost of the cap — our 2.3–5.0 points are new evidence |
| **D17** layout = hierarchy-constrained disc packing, deterministic | Uncharted: the same recursive circles-in-parent idea (no numbers); Voronoi/Gosper treemaps and Muelder–Ma fill the plane at similar or lower cost; NCP / NP-Voronoi treemaps seed children from a projection | **Ahead** on scale + determinism (no paper lays out a 1.8 B-edge graph deterministically; SNAP-tFDP is the only Friendster layout and is racy, 29.5 GB, no tiles). **Behind** on area and on meaning of distances; **unknown** on quality vs a real baseline (see Challenge 3) | Arithmetic from our parameters: children get φ = 0.72 of the parent area, leaves 0.5, area retention 0.946–0.955; a node at depth 7–9 gets 0.72⁶–0.72⁸ × 0.5 ≈ 3.6–6.9 % of the map before packing loss, the same order as the "1–3 %" estimate. **The loss compounds with depth by construction**; even NCP-level packing (0.87 envelope compactness) over 8 levels keeps < 0.87⁸ ≈ 33 %. Space-filling cells keep 100 % at every inner level |
| D17 quality claim "0.014 vs 0.244" | Noack normalized edge length (FA2 Eq. 15) is our ratio with means; NP_r, stress at optimal scale, edge-length CV | **Behind** on evaluation | Our own files: lj mean ratio 0.226 vs FastRP 0.264 (only 15 % apart); **lj p90 edge length 0.430 ≈ random-pair median 0.427**, orkut 0.475 vs 0.442 (`eval_lj_hcl.json`, `eval_orkut_hcl.json`): the longest 10 % of edges are as long as random pairs. The baseline was an 8-dim FastRP + PCA, not a published large-graph layout |
| D17 "pull to external neighbours" (no gain) | stress on the parent's quotient graph (exact MDS for ≤ 384 children, SGD stress, Hu's diameter-ratio scaling); NP-Voronoi treemaps: init-only similarity loses up to 50 % of neighbourhoods (citing Nocaj), so it must also be kept during optimisation | **Behind** | No paper does exactly "stress among sibling communities"; sparse stress (pivot regions ≈ communities, size-scaled weights s/d²) is the closest |
| **#67** layout never in heap, bounded memory | SNAP-tFDP, DRGraph, BatchLayout, sfdp: all hold the edge list in RAM; Muelder–Ma Θ(V) extra; ASK-GraphView disk antichains | **Ahead** in ambition, no prior art to copy | Tracer: mmap layout 405.1 s, 733 GB read under 16 GiB (§ M1-01) |
| **D18** data tiles in PMTiles v3, WebGL2 viewer | Uncharted: data tiles (Avro/HBase, served as images or JSON); deepscatter: Arrow quadtree tiles on static hosting; MSAGLJS: vector tiles through deck.gl; map-of-github: MapLibre vector tiles | **Level** with practice, **ahead** in single static file + exact aggregates + declared omissions | Our lj first view 152 KB / 18 requests; Friendster 163,184 B. No paper reports first-view bytes or omission accounting |
| **#68/#69** what a tile shows; importance quota per tile | GraphMaps/MSAGLJS: PageRank prefix with per-tile quota (MSAGLJS max tile 6–8× nominal); Uncharted: community hierarchy | **Level**; our hierarchy-driven choice matches Uncharted and the patent note in #68 | — |
| **D25** links for the selection only, strongest links optional, no bundling | GraphMaps: click shows all incident edges and neighbours; its one evaluating user preferred that on the dense Caltech graph; Uncharted: always-on intra/inter-community link heat-map layers with opacity fall-off for off-screen links | **Consistent with the evidence, untested at our scale** | Yoghourdjian 2021 thresholds; Saket Fig. 6 (link density lowers even node-task accuracy, 86.4 → 75.5 %); Jianu (~25 % accuracy cost on path tracing over group overlays); Elmqvist–Fekete G3/G6. **Gap:** no overview of inter-region connectivity without a selection (overview tasks dominate at > 1000 nodes, survey §V-A) |
| D25 bubbles + five reused region colours | GMap colours the country graph by SPECTRAL+GREEDY over a ColorBrewer ramp to maximise adjacent colour distance | **Level** (our "graph colouring of touching regions" in #68 is the same idea) | GMap §5 |
| **#26 / m1-12** viewer without discrete GPU | Embedding Atlas: 60 fps up to 4 M points, M1 Pro, WebGPU; cosmos.gl many-body needs `EXT_float_blend`, absent on iOS ≥ 15.4 | unmeasured on our side | vendor/paper figures, not comparable |
| **m9-05** GPU layout | GPU FA2, cuGraph, SNAP-tFDP GPU (com-lj 9.3 s) | not needed; all use atomics/races | keep deferred |

## 4. Challenges (ranked)

1. **Place siblings by stress on the parent's quotient graph, keep it during packing.**
   - *Change:* in each parent (≤ 384 children; 64 under the cap), target distance d_ij from
     inter-community affinity (e.g. −log of normalized cut weight, floored at
     r_i + r_j + gap); exact classical MDS init (deterministic up to sign — fix sign by a
     rule), then Zheng-style stress SGD with a seeded counter-based shuffle, weights scaled by
     child size (Ortmann's s/d²), then the existing overlap sweep. Children start from the
     parent's global coordinates of their quotient neighbours (Hu's diameter-ratio scaling).
   - *Expected gain:* the p90/mean of our edge-ratio — today p90 ≈ random — is what this
     targets; no paper measures it for nested layouts, so the gain is unknown. NCP's
     projection-seeded packing raised NP₁ 0.26 → 0.36 over greedy front-chain on ≤ 1083
     circles (one level).
   - *Cost/risk:* ~200 lines in the per-parent solve; O(k²) per parent ≈ the current force
     solve; deterministic if the shuffle is counter-based and per-parent sequential (parents
     in parallel, as now). No promise touched.
   - *Experiment:* interleaved A/B on lj and orkut (current FR solve vs stress), same
     hierarchy, 16 threads: mean, p90 and median edge ratio, NP₁/NP₂ on 10⁵ sampled nodes,
     stress of the top two quotient levels, time. *Issue:* #21 (`m1-07`, candidate 3 —
     sharpen it to this recipe) → #67.

2. **Compute the layout as power-diagram cells; draw cells or inscribed bubbles.**
   - *Change:* replace nested-disc area allocation with a Voronoi-treemap (power diagram,
     Lloyd + weight adaptation, Nocaj–Brandes) per parent, seeded by Challenge 1's positions;
     the viewer keeps the approved bubble style by drawing each child's largest inscribed
     circle (NCP does power-diagram → circles) or the "cells" style already in the preview.
   - *Expected gain:* inner levels fill 100 % instead of φ = 0.72 per level; by the
     arithmetic in § 3 finest-community area rises from ≲ 5 % to 100 %, which is above ADR
     0017's own "10× more of the map" trigger — **to be measured, not assumed**. Fewer zoom
     levels for the same node radius (zmax is set by the 10th-percentile node radius), so
     fewer tiles.
   - *Cost/risk:* a robust power diagram in Rust without C deps (estimate ~600–900 lines;
     power cells clipped to a convex parent are intersections of half-planes, so every
     level stays convex and clipping stays simple; fall back to the parent's inscribed disc
     if a cell degenerates). Floating point must be order-fixed for byte identity. Risk to D25's approved look: small if
     drawn as inscribed bubbles; Bubble Treemaps argue deliberate empty space helps reading,
     so measure, do not maximise blindly.
   - *Experiment:* lj and orkut, A/B: occupied-area fraction (node discs and finest
     regions), zmax, tile count and file size, edge ratios, time. *Issue:* #21 → D17 ADR.

3. **Measure our layout against one published large-graph layout before freezing D17.**
   - *Change:* none in the product; a baseline run.
   - *Why:* the D17 evidence compares with an 8-dim FastRP + PCA. On the mean ratio the gap
     is 0.226 vs 0.264. DRGraph has public code and ran com-Orkut in 18.4 GB on one thread;
     sfdp (EPL-2.0) and OGDF FM³ are the classical stress winners; BatchLayout is
     thread-invariant. Without this, "disc packing reads better" is the experimenter's
     visual judgment.
   - *Cost/risk:* a lab script, one run per tool under `MemoryMax=8G`–`24G`, lj only (and
     orkut where it fits); no product code.
   - *Metrics:* our three ratios, NP₁/NP₂ sampled, stress at optimal scale on sampled pairs
     (Smelser/Ahmed: raw normalized stress is scale-dependent), ARI/silhouette against
     SNAP's ground-truth `com-lj` communities (not against our own hierarchy — circular).
   - *Issue:* new, "m1-xx: layout baseline against published large-graph layouts", feeding
     #21.

4. **Make the layout's I/O sequential by laying out in hierarchy (DFS) order; place nodes
   of giant communities along a space-filling curve of that order.**
   - *Change:* the layout scans each parent's members' adjacency per depth. If node ids are
     the hierarchy DFS order (#23 / #146 relabel), every parent's members are a
     contiguous id range and every depth is one sequential pass. For finest communities
     above a size threshold, place nodes along a Hilbert/Gosper curve of their sub-order
     (Muelder–Ma: Θ(n), deterministic) instead of a force solve that gives near-uniform
     clouds today (`lab/e-map/REPORT.md` Q4).
   - *Expected gain:* reads become ≈ D × the store, ≈ 8 × 15.5 GB ≈ 124 GB sequential for
     Friendster raw CSR, against 733 GB of random re-reads in the tracer (arithmetic,
     extrapolation, not measured); upper levels from quotient graphs reduce it further.
   - *Cost/risk:* depends on the relabelling decision (#146); otherwise a sort of members per
     parent. Curve placement can only show order, not internal topology.
   - *Experiment:* Friendster under `MemoryMax=16G`, layout with and without hierarchy order:
     bytes read, time; on lj, edge ratio inside giant communities curve vs force. *Issue:*
     #67, link to #146 and `m1-09`.

5. **Keep "links for the selection only", add a connectivity overview that needs no
   selection.**
   - *Change:* for the current parent (or the top level), a reordered adjacency matrix of
     its children's quotient graph with exact weights, opened from the panel; per-region
     external-weight glyph (internal vs external, top partner); the "strongest links" layer
     always shows "showing X % of inter-region weight" (G5) and is drawn so a band does not
     look like an edge (G4). Bundling only for the selection's link set, along the hierarchy
     (Holten's own selection workflow).
   - *Evidence:* matrices beat node-link on dense graphs except paths (Ghoniem, via Okoe);
     overview tasks dominate at > 1000 nodes; selection-only cannot answer "is it modular
     or all-to-all" — our top quotient is nearly complete.
   - *Cost/risk:* the matrix of ≤ 64–384 children is already in the tile data the lab
     computes (aggregated edges); viewer work only. Changes nothing approved, adds a view.
   - *Experiment:* #20's task study (3–5 people): add the task "which regions are most
     tightly coupled" with and without the matrix. *Issue:* #20 (`m1-06`), `m5-07`.

6. **Adopt a scale-invariant metric suite and report distributions.** Median ratio stays;
   add mean and p90, NP₁/NP₂ (Jaccard of r-ball vs k_i nearest layout neighbours, tsNET
   §4.5) on sampled nodes, stress at optimal scale, coefficient of variation of edge length
   (Hachul–Jünger), occupied area, and per-level "sibling adjacency vs quotient weight".
   Expect NP ≤ 0.04 on social graphs for every method (t-FDP, SNAP-tFDP), so NP will not
   separate methods there: use it on the quotient levels. Cost: one evaluation script.
   *Issue:* acceptance of #21 and #67; ADR 0017's "what would change our mind" should name
   these.

7. **Use ASK-GraphView's recipe as a #19 candidate.** Recursive re-clustering of any
   cluster with > 64 children, with a structural fallback (peeling of tree-like periphery,
   biconnected decomposition) when re-clustering does not split a dense parent — exactly
   the "18.7 M children" case. Cost: a post-pass on the plain partition (fits the tracer's
   4.2 GiB solve). Experiment: #19's metrics on lj, orkut, Friendster. *Issue:* #19 / #63.

8. **Prior art for the patent check.** The Uncharted paper (Oct 2015) and Aperture Tiles
   (MIT) describe community hierarchy → recursive in-parent layout → data-tile pyramid →
   web map. Give both to the attorney with the `.egmap` spec (`m0-10`, `m8-10`). No legal
   conclusion drawn here.

9. **Tile payload as Arrow IPC** (deepscatter precedent: zero-parse, GPU-copyable) is
   already a comparison in #68; keep it. Low priority.

10. **GPU layout stays deferred** (`m9-05`): every GPU layout found uses atomics or
    lock-free races (GPU FA2, cuGraph, SNAP-tFDP), so none meets D14 as published.

## 5. What we have that the literature does not — novelty checked

| Claim | Verdict |
|---|---|
| Hierarchical zoomable map of a graph down to single nodes with data tiles | **Not new**: Uncharted 2015 (Louvain + recursive circles + data tiles + web map, 2.37 M nodes), ASK-GraphView 2006 (hierarchy navigation, 16 M edges), map-of-github (static vector tiles). New is the scale on one machine (1.8 B edges end to end) |
| Fan-out-capped hierarchy for the map | **Not new**: ASK-GraphView IL = 64. New: measured modularity cost of the cap, and typed artefact regions drawn as such |
| Deterministic, thread-count-invariant layout | **Partly new**: BatchLayout states thread invariance for FR; Hu fixes octree depth for reproducibility. New: byte-identical layout *and* tile file across thread counts and memory regimes at 10⁹ edges |
| Layout of a billion-edge graph under a memory budget | **New as far as found**: SNAP-tFDP did Friendster with 29.5 GB peak on a 220 GB machine; Graph Cities used another metaphor; nobody bounds memory |
| Exact aggregates at every level with declared omissions per tile | **New as far as found**: no system reports omitted weight per view; Elmqvist–Fekete G5 asks for it in principle |
| One static file (PMTiles v3) with search index for a graph map | **New for graphs** as far as found; deepscatter and map-of-github do static tiles for points |
| Hierarchy-driven tile content (not importance quota) | **Not new** (Uncharted); differs from GraphMaps/MSAGLJS |

## 6. What to ignore, and why

- tsNET / fast tsNET: O(N²) or ≤ 11k nodes in the papers; NP gain does not carry to social graphs.
- LargeVis: laid out a k-NN graph of embeddings, not edges; 9.26 h on LJ, nondeterministic.
- (SGD)²: ≤ 4096 nodes; only its node-resolution term is a useful idea.
- ForceAtlas2 as an engine: > 100k nodes "over several hours"; GPL in Gephi; keep only its metric.
- OpenOrd: subsumed by our hierarchy (coarsen-by-clustering); not accessible anyway.
- GraphMaps / MSAGLJS edge routing (CDT, sleeves): ≤ 38k nodes, hours; routed rails mislead (its own user).
- Edge-path / spectral bundling for the whole map: O(|E|² log |V|) or ≤ 88k edges, more ambiguity.
- GMap's Voronoi-with-random-points rendering: random boundaries, needs a global layout; take only its colouring.
- Graph Cities: edge-core buildings, not a node-link map; different product.
- BigGraphVis: GPU, 34 M edges in ~5 min, supernodes only.
- Topographic maps (Gronemann–Jünger): 2.5D relief on ≤ 10³ nodes; not accessible; aesthetic only.
- Embedding/UMAP of node embeddings as the map: our lab FastRP blob and LargeVis both say positions lose community containment.
