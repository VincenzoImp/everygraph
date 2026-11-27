# m1-18 — our map layout against published large-graph layouts (#175)

**Question.** D17's evidence compares our hierarchy-constrained disc packing with an
8-dimensional FastRP + PCA layout. How does it compare with published large-graph layouts
on the same graph, on metrics that do not depend on our own hierarchy?

**Answer.**
- **Ahead on local and community structure**, on lj and on orkut. In our layout, a node's
  neighbours, a community's strongest neighbours and SNAP's ground-truth communities are
  much closer together than in DRGraph or ForceAtlas2.
- **Behind on global structure.** Edges that leave a large region are as long as random
  pairs. Layout distance has no correlation with graph distance (Spearman 0.03 on both
  graphs, the same as a random layout; DRGraph 0.31 and 0.55).
- **Candidate (a), stress on the parent's quotient graph, closes part of the global gap on
  orkut** (Spearman 0.03 → 0.33, mean edge ratio 0.37 → 0.30). It does nothing global on lj,
  where each top region is linked to 44 of the other 70. It raises local scores on both
  graphs and costs about 1.8× the solve CPU.
- On orkut, starting the top one or three tree depths from DRGraph's community
  centroids gives about the same global gain (Spearman 0.30–0.31), but DRGraph costs
  919 s. On lj neither change helps. Rotating each sibling group to face its external neighbours gives no material
  gain on either graph.
- BatchLayout (Barnes-Hut, random start, 600 iterations) ran 80 minutes and did not get
  far from its random start (median edge ratio 0.80). FM³ and sfdp do not fit 24 GiB on lj.

Every number below is in a JSON file in this directory.

## What ran

All runs were on the shared lab machine (Ryzen 9 7950X, 61 GB). Other agents were
compiling and benchmarking, with load average 14–62. **All times and memory figures are
exploratory.** Each run used a hard limit,
`systemd-run --user --scope -p MemoryMax=<limit> -p MemorySwapMax=0`. The files are
`runs/<layout>_<graph>.json`: command, rc, wall, user and sys seconds, peak RSS from
getrusage, and peak anonymous RSS sampled every 0.25 s. Wall times include reading the
input: the binary CSR for ours, cuGraph and the metrics, and text edge lists for DRGraph,
FM3, BatchLayout and sfdp. The page cache was warm and there was no `fsync`.

| layout | what it is | version, options | threads | lj | orkut |
|---|---|---|---|---|---|
| `hcl` | ours: hierarchy-constrained disc packing, force solve per parent (`lab/e-map/gmap layout`) | φ = 0.72, 50 iterations, kmax 384; hierarchy `data/hier/<g>` (b2-leiden v2, map-shaped, fan-out cap 64) | 8 | ok, 8G | ok, 8G |
| `hcls` | ours with candidate (a): siblings placed by stress on the parent's quotient graph (`--solve stress`, `gmap/src/stress.rs`), then the same packing | target = shortest path with edge length r_i + r_j + gap·(1 + ln(a_max/a_ij)); classical MDS start; 30 SGD epochs, d⁻² weights | 8 | ok, 8G | ok, 8G |
| `hclo`, `hclso` | `hcl` / `hcls` plus `--orient`: each sibling group rotated (or mirrored) as one rigid body to face its children's external neighbours | closed-form best rotation | 8 | ok, 8G | ok, 8G |
| `hclg1`, `hclg3` | diagnostic: `hcl` with the children of the top 1 or 3 tree depths started at their centroids in DRGraph's layout (`--seed-pos`), then packed | — | 8 | ok, 8G | ok, 8G |
| `drgraph` | DRGraph (Zhu et al. 2020) | ZJUVAI/DRGraph@4f3fda2, README options `-neg 5 -samples 400 -gamma 0.1 -mode 1 -A 2 -B 1` | 8 | ok, 8G | ok, 24G |
| `fa2` | ForceAtlas2 in RAPIDS cuGraph 26.10, **on the GPU** (RTX 4090), a reference only | cuGraph defaults: 500 iterations, Barnes-Hut θ 0.5 | GPU | ok, 8G | ok, 12G |
| `fm3` | OGDF FMMMLayout (FM³), through `ogdf_fmmm.cpp` | OGDF foxglove-202510, high-level options, BeautifulAndFast | 1 | **killed at 16G (twice) and at 24G** | not tried |
| `sfdp` | Graphviz sfdp | 14.1.2 (conda-forge), `-Goverlap=true -Gsplines=false -Tplain` | 1 | **killed at 24G** after 193 s | not tried |
| `batchlayout` | BatchLayout (Rahman et al. 2020) | khaled-rahman/BatchLayout@d2e260d, `-algo 4` (Barnes-Hut), 600 iterations, `-init 1` (random start) | 8 | ok, 16G (4,795 s) | not tried |
| `random` | uniform in the unit square, seed 1: the floor for every metric | — | — | — | — |

- FM3 and sfdp do not fit the issue's 24 GiB ceiling on lj (35 M edges). FM3 used 24.1 GiB
  anonymous after 71 s. sfdp used 24.5 GiB after 193 s, while it was still reading the
  640 MB DOT file. Neither run reached a layout. A larger limit was not tried, because
  about nine agents shared the 61 GB machine.
- BatchLayout's default greedy start (`-init 0`) is killed at 8G and at 16G within 8 s
  (`runs/batchlayout_init0_16G_lj.json`). Its DFS start also keeps a 16 MB array on the
  stack, so it needs a larger `ulimit -s`. With the random start (`-init 1`), the run
  finished in 4,795 s with 4.2 GB. The layout is still close to random: its median edge
  ratio is 0.80 and its NP₁ is 0.001. This is BatchLayout without its own start, so it is not
  a fair picture of the method. It is kept in the tables as measured.
- DRGraph on orkut peaked at 18.4 GB anonymous. The DRGraph paper reports 18.4 GB for
  com-Orkut on one thread.
- FA2 runs on the GPU, and the product never needs a GPU. Its time is therefore not
  comparable with the CPU tools. It is here for its layout.

## Metrics (`metrics.py`, the same script for every layout)

- **Edge ratios** use all edges and 10⁷ random node pairs (seed 1). Median edge length ÷
  median random-pair distance is D17's metric. Also reported: mean ÷ mean, p90 edge ÷
  random-pair median (the framing in `docs/research/sota-maps.md`), p90 ÷ random p90, and
  the edge-length CV.
- **NP₁ / NP₂** is the Jaccard index of a node's 1-hop (2-hop) ball with the same number of
  nearest nodes in the drawing, on 10,000 sampled nodes. NP₂ uses the nodes whose 2-hop ball
  has ≤ 5,000 nodes: 9,437 on lj and 4,957 on orkut.
- **Quotient-level NP.** On each level of our hierarchy, a community is placed at the
  centroid of its members, in every layout. **NP@10** is the overlap of the 10 strongest
  quotient neighbours (w_ij / √(vol_i vol_j)) with the 10 nearest centroids. NP₁ over all
  quotient neighbours is in the JSON files. It means nothing on dense levels: the random
  layout scores 0.13 on lj L4 and 0.72 on lj L6.
- **Stress at the optimal scale** uses 100 BFS sources × 1,000 random targets, d⁻²
  weights, at the scale α* that minimizes it. **Spearman's ρ** between layout distance and
  hop distance uses the same pairs. With 100 sources, |ρ| ≤ about 0.03 is noise.
- **Ground truth** is SNAP's `com-<g>.top5000`, restricted to nodes in exactly one of these
  communities and to communities with at least 2 such nodes. That gives 52,592 nodes in
  3,320 communities on lj, and 494,179 nodes in 4,589 on orkut. The metrics are the
  silhouette of the labels in the drawing (on a 120,000-node sample for orkut), the 10-NN
  label agreement among labelled nodes (chance 0.002 on lj, 0.001 on orkut), and the ARI
  of k-means (k = number of communities) on their positions. For reference, the ARI of our
  hierarchy levels against the same labels, independent of any layout: lj best 0.445 (L3),
  orkut best 0.229 (L3).

## lj (4.0 M nodes, 34.7 M edges)

| layout | median ratio | mean ratio | p90 / random median | NP₁ | NP₂ | Spearman | stress at α* | GT silhouette | GT 10-NN | GT k-means ARI |
|---|---|---|---|---|---|---|---|---|---|---|
| hcl (ours) | **0.013** | 0.248 | 1.012 | 0.247 | 0.143 | 0.033 | 0.207 | −0.015 | 0.786 | 0.319 |
| hcls (ours + stress) | 0.013 | 0.261 | 1.069 | **0.335** | 0.149 | −0.032 | 0.217 | −0.006 | **0.792** | 0.317 |
| hclo (ours + orient) | 0.012 | 0.244 | 1.002 | 0.253 | 0.144 | 0.029 | 0.208 | −0.007 | 0.787 | 0.318 |
| hclso (ours + stress + orient) | 0.011 | 0.256 | 1.048 | 0.344 | 0.154 | −0.027 | 0.218 | 0.006 | 0.795 | 0.324 |
| hclg1 (ours, root from DRGraph) | 0.013 | 0.263 | 1.096 | 0.247 | 0.143 | 0.008 | 0.212 | −0.014 | 0.786 | 0.317 |
| hclg3 (ours, top 3 depths from DRGraph) | 0.013 | 0.259 | 1.074 | 0.250 | 0.143 | 0.007 | 0.214 | −0.014 | 0.786 | 0.317 |
| DRGraph | 0.079 | **0.133** | **0.387** | 0.015 | 0.018 | **0.311** | 0.312 | −0.220 | 0.553 | 0.306 |
| FA2 (GPU) | 0.149 | 0.141 | 0.475 | 0.012 | 0.020 | 0.167 | 0.471 | −0.226 | 0.515 | 0.187 |
| BatchLayout (random start) | 0.802 | 0.692 | 1.446 | 0.001 | 0.001 | 0.086 | 0.489 | −0.737 | 0.022 | 0.001 |
| random | 1.000 | 1.000 | 1.678 | 0.000 | 0.000 | 0.032 | 0.212 | −0.768 | 0.002 | 0.000 |

NP@10 on the quotient levels (L0 = 1,243,440 finest communities, L6 = 71 top regions):

| layout | L0 | L1 | L2 | L3 | L4 | L5 | L6 |
|---|---|---|---|---|---|---|---|
| hcl | 0.156 | 0.110 | 0.082 | 0.067 | 0.047 | 0.009 | 0.075 |
| hcls | **0.193** | **0.132** | **0.095** | **0.081** | **0.059** | 0.010 | 0.074 |
| DRGraph | 0.003 | 0.001 | 0.002 | 0.006 | 0.013 | 0.013 | **0.202** |
| FA2 | 0.004 | 0.002 | 0.003 | 0.006 | 0.013 | 0.012 | **0.211** |
| random | 0.000 | 0.000 | 0.000 | 0.000 | 0.001 | 0.004 | 0.115 |

## orkut (3.1 M nodes, 117.2 M edges)

| layout | median ratio | mean ratio | p90 / random median | NP₁ | NP₂ | Spearman | stress at α* | GT silhouette | GT 10-NN | GT k-means ARI |
|---|---|---|---|---|---|---|---|---|---|---|
| hcl (ours) | 0.088 | 0.369 | 1.386 | 0.102 | 0.054 | 0.035 | 0.222 | −0.785 | 0.524 | **0.199** |
| hcls (ours + stress) | 0.093 | 0.298 | 0.956 | **0.115** | **0.055** | 0.326 | **0.182** | −0.780 | **0.533** | 0.197 |
| hclso (ours + stress + orient) | 0.085 | 0.287 | 0.927 | 0.118 | 0.056 | 0.331 | 0.180 | −0.776 | 0.535 | 0.201 |
| hclg1 (ours, root from DRGraph) | 0.090 | 0.315 | 1.082 | 0.103 | 0.054 | 0.305 | 0.182 | −0.776 | 0.524 | 0.197 |
| hclg3 (ours, top 3 depths from DRGraph) | 0.087 | 0.304 | 1.032 | 0.102 | 0.054 | 0.296 | 0.184 | −0.774 | 0.524 | 0.198 |
| DRGraph | **0.070** | 0.103 | 0.263 | 0.002 | 0.012 | **0.547** | 0.294 | −0.784 | 0.036 | 0.015 |
| FA2 (GPU) | 0.076 | **0.100** | **0.241** | 0.002 | 0.011 | 0.521 | 0.344 | −0.788 | 0.030 | 0.010 |
| random | 1.000 | 1.000 | 1.678 | 0.000 | 0.000 | 0.020 | 0.209 | −0.948 | 0.001 | 0.000 |

NP@10 on the quotient levels (L0 = 722,593, L6 = 84; L7 has 14 communities, too few to
score):

| layout | L0 | L1 | L2 | L3 | L4 | L5 | L6 |
|---|---|---|---|---|---|---|---|
| hcl | 0.088 | 0.086 | 0.094 | 0.093 | 0.079 | 0.051 | 0.045 |
| hcls | **0.108** | **0.101** | **0.105** | **0.102** | 0.080 | 0.054 | 0.055 |
| DRGraph | 0.001 | 0.002 | 0.013 | 0.044 | **0.085** | **0.083** | 0.063 |
| FA2 | 0.001 | 0.003 | 0.013 | 0.040 | 0.069 | 0.076 | 0.063 |
| random | 0.000 | 0.000 | 0.000 | 0.001 | 0.006 | 0.022 | 0.104 |

On orkut's L6 the quotient graph has a mean degree of 3.3, so NP@10 compares about
3 neighbours. Every layout is at or below random there.

## Where the long edges come from (`crosslevel_<g>.json`)

The table gives the median length of the edges that cross between level-k communities,
÷ the layout's random-pair median:

| graph, level | communities | share of edges crossing | hcl | hcls | hclo | hclso | hclg3 | DRGraph | FA2 |
|---|---|---|---|---|---|---|---|---|---|
| lj L0 | 1,243,440 | 0.829 | 0.051 | 0.050 | 0.048 | 0.048 | 0.049 | 0.087 | 0.171 |
| lj L4 | 5,512 | 0.437 | 0.347 | 0.357 | 0.328 | 0.347 | 0.339 | 0.131 | 0.245 |
| lj L6 | 71 | 0.210 | 0.981 | 1.037 | 0.970 | 1.017 | 1.041 | 0.233 | 0.321 |
| orkut L0 | 722,593 | 0.919 | 0.118 | 0.126 | 0.111 | 0.114 | 0.115 | 0.072 | 0.081 |
| orkut L4 | 1,031 | 0.538 | 0.430 | 0.435 | 0.433 | 0.416 | 0.419 | 0.100 | 0.106 |
| orkut L6 | 84 | 0.243 | 1.271 | 0.876 | 1.228 | **0.844** | 0.958 | 0.162 | 0.151 |

In our layout, an edge inside a finest community is 0.000–0.004 (lj) or 0.001–0.030
(orkut) of a random pair. In DRGraph it is 0.046–0.061. Take the 10 largest top regions:
on lj, DRGraph's are spread out (members at a median 0.34 of a random pair from their
centroid; ours 0.13). On orkut they are as compact as ours (0.14 against 0.12), yet
DRGraph's crossing edges stay at 0.16 (`top_level_10_largest`). So compact regions alone do
not force long crossing edges. Our layout loses on where, inside each region, the members
with outside links sit. A rigid rotation per sibling group (`hclo`) is too coarse to fix
that. This is a hypothesis; it was not measured.

## Cost (lj, exploratory, shared machine)

| layout | wall | peak anonymous | limit |
|---|---|---|---|
| hcl, 8 threads | 4.2–7.1 s (3 interleaved runs, `ab_lj.jsonl`) | 0.7 GB | 8G |
| hcls, 8 threads | 5.6–6.7 s (same runs) | 0.7 GB | 8G |
| DRGraph, 8 threads | 700 s (text read and multilevel 65 s, layout 623 s); orkut 919 s | 7.0 GB; orkut 18.4 GB | 8G; 24G |
| FA2, GPU | 26 s (layout 18 s); orkut 30 s | 1.3 GB host | 8G |
| BatchLayout, 8 threads, random start | 4,795 s | 4.2 GB | 16G |
| FM3, 1 thread | killed after 71 s | > 24 GB | 24G |
| sfdp, 1 thread | killed after 193 s | > 24 GB | 24G |

The ratio comes from the interleaved A/B on lj (`ab_lj.jsonl`, 8 threads, same machine
state). In rounds 1–2 the stress solve costs 1.8× the force solve's CPU (22.9–23.2 s
against 12.6–12.9 s), and 1.3× the layout's wall time. Round 3 was disturbed by load.
`hcls` is byte-identical at 3 and 8 threads on lj.

## Reading the numbers

1. **Local: ours is far ahead.** NP₁ is 0.25 against 0.015 on lj, and 0.10 against 0.002 on
   orkut. The papers' "NP ≤ 0.04 on social graphs" holds for DRGraph and FA2 here, but not
   for our layout, which draws each finest community as a tiny disc.
2. **Communities: ours is ahead on the fine and middle levels, and on ground truth.**
   NP@10 is 4–50× DRGraph's on lj L0–L4, and 2–90× on orkut L0–L3. The 10-NN agreement with
   SNAP's communities is 0.79 against 0.55 (lj), and 0.52 against 0.04 (orkut). The k-means
   ARI is level on lj (0.32 vs 0.31) and 13× on orkut (0.20 vs 0.015).
3. **Global: ours is behind.** The mean ratio is 0.25 against 0.13 (lj), and 0.37 against
   0.10 (orkut). The p90 edge is as long as a random pair. Spearman is 0.03 on both graphs,
   the same as random. On lj L6 the top regions are placed below random. On orkut even the
   median ratio is behind (0.088 against 0.070).
4. **The global gap is set by the edges between large regions.** 21% (lj) and 24% (orkut)
   of edges cross the top regions. In our layout they are as long as random pairs (0.98 and
   1.27); in DRGraph they are 0.23 and 0.16.
5. **Candidate (a) helps where the quotient graph has structure.** On orkut it brings
   Spearman from 0.03 to 0.33, the p90 edge from 1.39 to 0.96 of a random pair, and the top
   crossing edges from 1.27 to 0.88. On lj, where the top quotient graph is nearly
   complete, it brings no global gain. On both graphs it raises NP₁ (+36% lj, +13% orkut)
   and NP@10 on the fine levels, for 1.8× the solve CPU.
6. **Upper-level placement is the lever, where there is one.** On orkut, starting the
   root's children (`hclg1`) or the top three depths (`hclg3`) from DRGraph's centroids
   brings Spearman to 0.31 or 0.30 and the mean ratio to 0.32 or 0.30. That is about what
   stress reaches from the quotient graph alone. On lj neither changes the mean ratio by
   more than 0.015. Rotating sibling groups (`hclo`) moves the
   mean ratio by ≤ 0.005 on lj and does nothing on orkut's top crossing edges
   (1.27 → 1.23).
7. **Stress does not separate these layouts.** The random layout scores 0.21 on both
   graphs, ours 0.18–0.22, DRGraph 0.29–0.31. On a small-world graph almost all pairs are
   4–7 hops apart, so a drawing with nearly constant distances scores well. Spearman's ρ is
   the better test of global distance. The ground-truth silhouette does not separate the
   layouts on orkut either: −0.78 for all of them, because the communities are large and
   overlap.
8. **ADR 0017's trigger "edge-length ratio on orkut below 0.05" is out of reach for every
   layout tested:** ours 0.088, DRGraph 0.070, FA2 0.076.

## Recommendation for D17

**Keep disc packing in hierarchy regions; adopt candidate (a); narrow the claims; measure
the remaining global gap in #21.**
- **Keep.** No published layout tested draws the communities. Ours puts nodes near their
  neighbours, communities near their strongest neighbours, and ground-truth communities
  together. It runs in seconds, uses under 1.2 GB on lj and orkut, and is deterministic. The
  published CPU tools take 700–919 s (DRGraph), 4,795 s without converging (BatchLayout,
  random start), or do not fit 24 GiB (FM3, sfdp).
- **Change the per-parent solve to stress on the quotient graph.** It is deterministic,
  and it costs about 1.8× the solve CPU (a few seconds on lj). It raises NP₁ and NP@10 on
  both graphs; the only local loss is orkut's median edge ratio (0.093 against 0.088). On
  orkut it closes about a third of the global gap: Spearman goes from 0.03 to 0.33, against
  DRGraph's 0.55. That is as much as a start from DRGraph's own layout gives, without the
  919 s. The lab version is `lab/e-map/gmap/src/stress.rs` (150 lines). #67 / m5-01 should
  take it; #21 answered the area question (`lab/m1-07-layout-area/results/README.md`).
- **Narrow the claims in D17 and ADR 0017.** Distances between large regions carry little
  or no information: Spearman 0.03, with the top crossing edges as long as random pairs. Do
  not judge D17 on the mean or p90 edge ratio alone. Replace the orkut-median trigger,
  which no layout reaches. Use instead, each against DRGraph as the published reference:
  NP@10 on the quotient levels, ground-truth 10-NN agreement, Spearman, and the length of
  edges between top regions.
- **Measure further (#21).** The remaining gap is probably in where each region's members
  sit relative to the region's outside neighbours. That is a hypothesis: on orkut, L6
  crossing edges are 0.88 of a random pair for us against 0.16 for DRGraph, with equally
  compact regions. The viewer must show inter-region
  connectivity (D25's gap), because geometry alone does not today.

## What was not measured

- FM3 and sfdp on lj above 24 GiB, and on orkut. BatchLayout with its default greedy start
  (killed), and BatchLayout on orkut.
- Any absolute time on an idle machine. The official command is in the PR.
- M1-05's `rec` hierarchy: no lj or orkut copy was on disk, so every run used
  `data/hier/<g>`.
- Images. No visual comparison was made.
- Within-region placement towards outside neighbours, beyond a rigid rotation.

## Reproduce

`../run_all.sh` runs the steps setup, layouts, ab and metrics. The scratch directory is
`/home/tux/graphlab/scratch/m1-18-layout-baseline`.
