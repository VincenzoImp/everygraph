# m1-07 — layout area and the global gap (#21)

**Questions.**
1. Nested discs leave nodes a small part of the map. How much, and does a layout with the
   same cost and determinism use the plane better?
2. ADR 0017's open item: edges between top regions are as long as random pairs. Does the
   gap come from where members with outside links sit inside each region?
3. Which layout should m5-01 (#67) build?

**Answers.**
1. **Measured: node discs cover 1.7–1.9% of the unit-square map, finest regions 3.2–3.8%.**
   Each depth keeps 0.63–0.65 of its parent's area, so the loss compounds over 7–8 depths.
   **Power-diagram cells** (`cells`) give every child a convex cell with area ∝ its size.
   The cells fill 100% of each parent, and the drawn bubble (each cell's largest inscribed
   circle) keeps 0.98–0.99 of the parent's bubble per depth. Node discs then cover 64–66% of
   the map (39–40× `hcls`) and finest regions 68–71% (21–22×). The 10th-percentile node
   radius grows 5.3–5.9×, the tile stage's zmax drops from 10 to 7, and the tile set has
   8–11× fewer tiles and is 3.1–3.4× smaller. This is far above ADR 0017's "10× more of the
   map" trigger. It is deterministic (byte-identical at 1 and 8 threads on lj and orkut),
   and no cell fell back to discs.
   **But it costs more and reads worse.** At 1 thread it takes 2.4–2.9× the time of `hcls`
   (exploratory). It lowers every local and community score on both graphs: NP₁ 0.335 →
   0.197 (lj) and 0.115 → 0.093 (orkut), ground-truth 10-NN 0.792 → 0.755 and 0.533 → 0.493,
   NP@10 lower at every level on lj and at L0–L5 on orkut. On orkut it also lowers Spearman
   (0.326 → 0.229) and lengthens edges between top regions (0.876 → 0.924 of a random pair).
   Without empty space between regions, a node's nearest neighbours on the map include nodes
   of the next region. Its nodes fill the whole cell, so they also fall outside the bubble
   drawn for that cell: it fits a "cells" view, not the approved bubble style.
   A bubble-compatible variant (`cellsd`: cells inside each parent's bubble, so bubbles nest)
   keeps 0.73–0.74 per depth: node discs 7.7–8.9% of the map (4.7–5.4× `hcls`), below the
   10× trigger, at 3.3–5.4× the time of `hcls` in this lab code, and with lower scores than
   `hcls` too (NP₁ 0.286 lj, 0.109 orkut; Spearman 0.306 orkut).
2. **No. The hypothesis does not hold.** Edges between top regions are as long as the
   distance between their regions' centroids (lj: 1.04 against 1.01 of a random pair for
   `hcls`; orkut: 0.88 against 0.82). Where a member sits inside its region changes little:
   - The members' outside links point mostly the same way as their region's links as a
     whole. A region's members average to its centroid, so that common direction cannot be
     faced. Without it, a level-k community's links can face at most 0.02 (L5), 0.07 (L4),
     0.10 (L3) and 0.13 (L2) of the region's radius on lj, and 0.03, 0.04, 0.06 and 0.08 on
     orkut. The coarse levels set most of a member's offset, and there the bound is smallest.
   - A placement that pulls each child towards its outside neighbours inside every parent
     (`hclsb`, the cheapest variant of (b)) does move children: 0.12 of the radius at depths
     2–3 on lj, from the tree's disc centres (`depth_facing_lj.txt`). But each region's
     centroid moves with them, so the facing measured from the centroid stays at 0.02 (lj)
     and 0.00 (orkut). A 3×, 10× or 100× stronger pull changes nothing (`gap_extra_<g>.json`).
   - On the four ADR 0017 measures `hclsb` is not better than `hcls`: top-region edges 1.00
     against 1.04 (lj) and 0.895 against 0.876 (orkut); Spearman −0.02 against −0.03 (lj) and
     0.330 against 0.326 (orkut); NP₁, ground-truth 10-NN and NP@10 at L0–L3 lower on both graphs.
   **The gap is in which regions are neighbours, and in how compact they are.** On lj an
   edge between top regions goes to a region that ranks 46th of 70 by centroid distance
   (median, `hcls`); only 2.6% go to one of the 3 nearest. With each top region linked to 44
   of the other 70, no placement of compact regions can make most of these pairs adjacent.
   DRGraph reaches 0.23 there by not keeping regions compact (members 0.34 of a random pair
   from their centroid, against 0.13). On orkut the rim-to-rim floor is lower (0.18 for
   `hcls`), so a better placement of the top regions themselves, not of their members, is
   the lever there; m1-18 showed that seeding the top levels from DRGraph's centroids
   brings about as much as stress already does.
3. **m5-01 builds `hcls`: nested discs, stress per parent, no pull towards outside
   neighbours, and no cells.** Details under "Recommendation".

Every number below is in a file in this directory.

## What ran

Code: `lab/e-map/gmap` (`layout --solve stress` as in m1-18, plus `--boundary B`, `--cells`,
`--cells-in-disc`, `--lloyd N`; `src/cells.rs`, `src/stress.rs`, `src/layout.rs`). Same
hierarchy as m1-18 (`data/hier/<g>`, b2-leiden v2, map-shaped, fan-out cap 64), φ = 0.72,
kmax 384, seed 1.

| layout | what it is |
|---|---|
| `hcl` | current packing: force solve per parent, nested discs (m1-18's `hcl`) |
| `hcls` | packing + stress solve per parent (m1-18's `hcls`, ADR 0017 amendment) |
| `hclsb` | (b) `hcls` plus boundary-aware placement: in each parent, after the stress layout, the best rigid rotation or mirror towards the children's outside neighbours, then 20 stress epochs with a spring from each child to the mean position of its outside neighbours' regions, of strength B · (outside share of its edges), B = 1; children kept inside the parent disc. `hclsb3`, `hclsb10`, `hclsb100`: B = 3, 10, 100 |
| `cells` | (a) power-diagram cells: in each parent, children start from the `hcls` positions, then 20 Lloyd steps with weight adaptation (Nocaj–Brandes: w ← w · target / area, capped by the squared distance to the nearest other site so every site stays in its own cell), all in f64, sequential per parent. The map is the unit square. Nodes sit at the centre of their own cell's largest inscribed circle. A degenerate cell would fall back to the `hcls` discs inside the parent's inscribed circle; none did |
| `cellsb` | `cells` started from `hclsb` |
| `cellsd` | `cells`, but each parent's cells fill its bubble (a 48-gon of its largest inscribed circle) instead of its cell, so that every bubble lies inside its parent's bubble |

Metrics: m1-18's `metrics.py` and `crosslevel.py`, unchanged, with the same seeds; DRGraph
and random rows are m1-18's files (same hierarchy, same metric scripts). Our `hcl` and `hcls`
rows reproduce m1-18's metric files exactly on lj and orkut, so the comparison is like with
like. New scripts: `gap.py` (decomposition of edges between top regions), `facing.py`
(how far members could face their outside links), `depth_facing.py` (at which tree depth
facing appears), `table.py` (the tables below).

Conditions: shared machine (Ryzen 9 7950X, 61 GB; load average 6–147 during the runs, other
agents compiling and benchmarking), every run under `systemd-run --user --scope -p
MemoryMax=8G` (layouts) or `12G` (metrics) `-p MemorySwapMax=0`. Layouts at 8 threads unless
stated; positions f32, layout arithmetic f32 (discs, stress) and f64 (cells); input the
binary CSR, page cache warm, no `fsync`. **Every time is exploratory.** Every quality
number is deterministic.

## 1. Area

Fractions of the unit-square map (the tile world). "Bubbles" are the drawn circles: the
nested discs, or each cell's largest inscribed circle. "Node discs" are the drawn node
circles. "Retention" is a depth's bubble area over its parents' bubble area. zmax is the
zoom at which the 10th-percentile node radius reaches 4 px on 256-px tiles (the rule of
`gmap tiles`, applied per node). For cells, every inner level owns 100% of its parent
(`cells` field of `by_depth` in the JSON); "misallocated" is the share of a parent's area
given to the wrong child, over all parents with two or more children.

| graph | layout | node discs | finest-region bubbles | bubbles at depth 1–7 | mean retention per depth | node radius p10 | zmax | cell area misallocated: mean / p90 / max | fallbacks | peak RSS MB (8 threads) |
|---|---|---|---|---|---|---|---|---|---|---|
| lj | hcl | 0.019 | 0.036 | 0.53 0.32 0.21 0.13 0.08 0.05 0.03 | 0.64 | 2.8e-05 | 10 | — | — | 828 |
| lj | hcls | 0.017 | 0.032 | 0.52 0.31 0.20 0.13 0.08 0.05 0.03 | 0.63 | 2.5e-05 | 10 | — | — | 830 |
| lj | hclsb | 0.014 | 0.027 | 0.52 0.30 0.18 0.11 0.07 0.04 0.02 | 0.61 | 2.1e-05 | 10 | — | — | 830 |
| lj | cells | 0.640 | 0.675 | 0.77 0.68 0.70 0.61 0.58 0.57 0.50 | 0.98 | 1.3e-04 | 7 | 0.0017 / 0.0041 / 0.48 | 0 | 1031 |
| lj | cellsb | 0.637 | 0.676 | 0.73 0.69 0.71 0.62 0.59 0.58 0.51 | 0.98 | 1.3e-04 | 7 | 0.0027 / 0.0056 / 0.63 | 0 | 1032 |
| lj | cellsd | 0.089 | 0.124 | 0.62 0.42 0.30 0.21 0.15 0.11 0.07 | 0.73 | 2.9e-05 | 10 | 0.0005 / 0.0008 / 0.38 | 0 | 1236 |
| orkut | hcl | 0.019 | 0.038 | 0.54 0.35 0.22 0.15 0.09 0.06 0.04 | 0.65 | 3.4e-05 | 9 | — | — | 1239 |
| orkut | hcls | 0.017 | 0.033 | 0.52 0.33 0.21 0.13 0.08 0.05 0.03 | 0.64 | 3.0e-05 | 10 | — | — | 1241 |
| orkut | hclsb | 0.014 | 0.028 | 0.54 0.34 0.21 0.13 0.08 0.05 0.03 | 0.62 | 2.6e-05 | 10 | — | — | 1241 |
| orkut | cells | 0.658 | 0.711 | 0.71 0.73 0.68 0.67 0.65 0.64 0.64 | 0.99 | 1.8e-04 | 7 | 0.0023 / 0.0058 / 0.45 | 0 | 1355 |
| orkut | cellsb | 0.652 | 0.703 | 0.71 0.72 0.67 0.66 0.64 0.64 0.62 | 0.99 | 1.3e-04 | 7 | 0.0057 / 0.0101 / 0.52 | 0 | 1358 |
| orkut | cellsd | 0.077 | 0.107 | 0.59 0.43 0.31 0.23 0.16 0.12 0.09 | 0.74 | 3.9e-05 | 9 | 0.0008 / 0.0013 / 0.39 | 0 | 1438 |

The arithmetic of challenge 2 (`docs/research/sota-maps.md`) expected 3.6–6.9% of the map
for a node at depth 7–9 before packing loss. Measured with packing loss: 1.7–1.9% for node
discs, 3.2–3.8% for finest regions.

Images (600 px, nodes coloured by top region, bubble outlines): `img/hcls_lj.png`,
`img/cells_lj.png`, `img/hcls_orkut.png`, `img/cells_orkut.png`, and a zoom into one lj
region with cells, `img/cells_lj_zoom.png`. In the cells images, the black around node
dots is the renderer drawing nodes at 0.55 of their radius, not unused area.

## 2. The four ADR 0017 measures, against DRGraph

### lj (4.0 M nodes, 34.7 M edges)

| layout | Spearman | top-region edges (L6, median ÷ random pair) | GT 10-NN | median ratio | mean ratio | p90 ÷ random median | NP₁ | NP₂ | GT k-means ARI |
|---|---|---|---|---|---|---|---|---|---|
| hcl | 0.033 | 0.981 | 0.786 | 0.013 | 0.248 | 1.012 | 0.247 | 0.143 | 0.319 |
| hcls | -0.032 | 1.037 | 0.792 | 0.013 | 0.261 | 1.069 | 0.335 | 0.149 | 0.317 |
| hclsb | -0.023 | 0.999 | 0.787 | 0.012 | 0.254 | 1.032 | 0.278 | 0.144 | 0.315 |
| cells | -0.032 | 1.024 | 0.755 | 0.025 | 0.274 | 1.055 | 0.197 | 0.112 | 0.255 |
| cellsb | -0.018 | 0.993 | 0.747 | 0.025 | 0.269 | 1.024 | 0.174 | 0.110 | 0.300 |
| cellsd | -0.032 | 1.030 | 0.783 | 0.016 | 0.267 | 1.061 | 0.286 | 0.143 | 0.295 |
| DRGraph (m1-18) | 0.311 | 0.233 | 0.553 | 0.079 | 0.133 | 0.387 | 0.015 | 0.018 | 0.306 |
| random (m1-18) | 0.032 | 1.001 | 0.002 | 1.000 | 1.000 | 1.678 | 0.000 | 0.000 | 0.000 |

NP@10 per quotient level

| layout | L0 | L1 | L2 | L3 | L4 | L5 | L6 |
|---|---|---|---|---|---|---|---|
| hcl | 0.156 | 0.110 | 0.082 | 0.067 | 0.047 | 0.009 | 0.075 |
| hcls | 0.193 | 0.132 | 0.095 | 0.081 | 0.059 | 0.010 | 0.074 |
| hclsb | 0.158 | 0.112 | 0.085 | 0.071 | 0.047 | 0.010 | 0.083 |
| cells | 0.132 | 0.095 | 0.075 | 0.063 | 0.044 | 0.008 | 0.058 |
| cellsb | 0.107 | 0.079 | 0.063 | 0.055 | 0.033 | 0.008 | 0.056 |
| cellsd | 0.178 | 0.123 | 0.091 | 0.074 | 0.057 | 0.008 | 0.069 |
| DRGraph (m1-18) | 0.003 | 0.001 | 0.002 | 0.006 | 0.013 | 0.013 | 0.202 |
| random (m1-18) | 0.000 | 0.000 | 0.000 | 0.000 | 0.001 | 0.004 | 0.115 |

### orkut (3.1 M nodes, 117.2 M edges)

| layout | Spearman | top-region edges (L6, median ÷ random pair) | GT 10-NN | median ratio | mean ratio | p90 ÷ random median | NP₁ | NP₂ | GT k-means ARI |
|---|---|---|---|---|---|---|---|---|---|
| hcl | 0.035 | 1.271 | 0.524 | 0.088 | 0.369 | 1.386 | 0.102 | 0.054 | 0.199 |
| hcls | 0.326 | 0.876 | 0.533 | 0.093 | 0.298 | 0.956 | 0.115 | 0.055 | 0.197 |
| hclsb | 0.330 | 0.895 | 0.522 | 0.089 | 0.302 | 0.972 | 0.103 | 0.054 | 0.197 |
| cells | 0.229 | 0.924 | 0.493 | 0.135 | 0.330 | 1.019 | 0.093 | 0.047 | 0.149 |
| cellsb | 0.244 | 0.903 | 0.481 | 0.123 | 0.320 | 0.999 | 0.081 | 0.046 | 0.145 |
| cellsd | 0.306 | 0.908 | 0.524 | 0.109 | 0.319 | 1.003 | 0.109 | 0.054 | 0.190 |
| DRGraph (m1-18) | 0.547 | 0.162 | 0.036 | 0.070 | 0.103 | 0.263 | 0.002 | 0.012 | 0.015 |
| random (m1-18) | 0.020 | 0.999 | 0.001 | 1.000 | 1.000 | 1.678 | 0.000 | 0.000 | -0.000 |

NP@10 per quotient level

| layout | L0 | L1 | L2 | L3 | L4 | L5 | L6 |
|---|---|---|---|---|---|---|---|
| hcl | 0.088 | 0.086 | 0.094 | 0.093 | 0.079 | 0.051 | 0.045 |
| hcls | 0.108 | 0.101 | 0.105 | 0.102 | 0.080 | 0.054 | 0.055 |
| hclsb | 0.089 | 0.091 | 0.097 | 0.095 | 0.085 | 0.045 | 0.057 |
| cells | 0.081 | 0.075 | 0.078 | 0.071 | 0.061 | 0.043 | 0.059 |
| cellsb | 0.064 | 0.066 | 0.072 | 0.068 | 0.060 | 0.045 | 0.055 |
| cellsd | 0.102 | 0.094 | 0.098 | 0.094 | 0.076 | 0.051 | 0.067 |
| DRGraph (m1-18) | 0.001 | 0.002 | 0.013 | 0.044 | 0.085 | 0.083 | 0.063 |
| random (m1-18) | 0.000 | 0.000 | 0.000 | 0.001 | 0.006 | 0.022 | 0.104 |

Reading:
- `hcls` stays the best of our layouts on the local and community measures on both graphs,
  and (with `hclsb`) on Spearman and top-region edges on orkut.
- `cells` and `cellsb` lose on every local and community measure. They remain far ahead of
  DRGraph there, but m5-01 requires "none worse than `hcls`".
- `cellsd` loses less, but still loses (lj NP₁ 0.286, NP@10 0.178 at L0 against 0.193).
- No variant moves Spearman or top-region edges on lj, where every one stays at random level.

## 3. Where the length of edges between top regions comes from

`gap.py`, level 6 (lj 71 regions, 21% of edges; orkut 84 regions, 24% of edges). L is the
median edge length; C the median distance between the centroids of the two regions;
G = max(0, C − R_A − R_B) with R = √2 · rms member distance to the centroid: what would be left
if every member with an outside link sat on the facing rim. All ÷ the random-pair median.

| graph | layout | L median | C median | G median | facing | share to 3 nearest regions | median rank of the other region |
|---|---|---|---|---|---|---|---|
| lj | hcl | 0.981 | 0.954 | 0.608 | 0.016 | 0.009 | 35 of 70 |
| lj | hcls | 1.037 | 1.013 | 0.682 | 0.006 | 0.026 | 46 of 70 |
| lj | hclsb | 0.999 | 0.975 | 0.676 | 0.020 | 0.012 | 40 of 70 |
| lj | cells | 1.024 | 1.010 | 0.546 | 0.007 | 0.015 | 43 of 70 |
| lj | cellsb | 0.993 | 1.000 | 0.503 | 0.023 | 0.014 | 40 of 70 |
| lj | cellsd | 1.030 | 1.014 | 0.635 | 0.004 | 0.020 | 44 of 70 |
| lj | hclsb3 | 0.997 | 0.975 | 0.680 | 0.025 | 0.013 | 41 of 70 |
| lj | hclsb10 | 0.995 | 0.972 | 0.678 | 0.024 | 0.013 | 41 of 70 |
| lj | hclsb100 | 0.995 | 0.971 | 0.678 | 0.023 | 0.013 | 40 of 70 |
| orkut | hcl | 1.271 | 1.353 | 0.643 | 0.025 | 0.151 | 48 of 83 |
| orkut | hcls | 0.876 | 0.818 | 0.180 | 0.033 | 0.074 | 44 of 83 |
| orkut | hclsb | 0.895 | 0.807 | 0.184 | -0.002 | 0.084 | 42 of 83 |
| orkut | cells | 0.924 | 0.794 | 0.107 | 0.018 | 0.075 | 35 of 83 |
| orkut | cellsb | 0.903 | 0.862 | 0.107 | 0.027 | 0.086 | 38 of 83 |
| orkut | cellsd | 0.908 | 0.892 | 0.289 | 0.030 | 0.169 | 22 of 83 |
| orkut | hclsb3 | 0.894 | 0.828 | 0.192 | 0.011 | 0.084 | 42 of 83 |
| orkut | hclsb10 | 0.888 | 0.832 | 0.178 | 0.010 | 0.084 | 43 of 83 |
| orkut | hclsb100 | 0.883 | 0.833 | 0.184 | 0.009 | 0.084 | 43 of 83 |

`facing.py` (positions of `hcls`): for groups of members, the most that they could face
their outside links (in units of the region's radius), raw and with the region's common
direction removed. "Chance" is the raw value for groups of the same sizes with shuffled
directions.

| graph | group | raw bound | chance | bound without the region's common direction |
|---|---|---|---|---|
| lj | node | 0.753 | 0.250 | 0.280 |
| lj | L0 | 0.723 | 0.161 | 0.223 |
| lj | L1 | 0.697 | 0.084 | 0.167 |
| lj | L2 | 0.681 | 0.042 | 0.127 |
| lj | L3 | 0.671 | 0.020 | 0.096 |
| lj | L4 | 0.665 | 0.010 | 0.072 |
| lj | L5 | 0.657 | 0.002 | 0.023 |
| orkut | node | 0.802 | 0.148 | 0.217 |
| orkut | L0 | 0.777 | 0.068 | 0.152 |
| orkut | L1 | 0.762 | 0.027 | 0.105 |
| orkut | L2 | 0.755 | 0.012 | 0.076 |
| orkut | L3 | 0.751 | 0.005 | 0.056 |
| orkut | L4 | 0.749 | 0.002 | 0.039 |
| orkut | L5 | 0.748 | 0.001 | 0.026 |

`depth_facing.py` (`depth_facing_<g>.txt`): the mean step towards the other region at each
tree depth, from tree disc centres. Depth 1 (lj) is the region's own place on the map. For
`hclsb` the steps at depths 2–3 are 0.05 and 0.06 of the radius on lj, against 0.01 and 0.00
for `hcls`; measured from the members' centroid instead, the facing is 0.02, because the
centroid moves with the members.

## 4. Determinism and cost

`determinism.txt`: positions (`pos.f32`), tree (`tnodes.bin`) and order (`order.u32`) are
byte-identical at 1 and 8 threads for all six layouts on lj and orkut (36 of 36 files).

Layout time at 1 thread ÷ that of `hcls` (`determinism.txt`; runs back to back on a shared machine, exploratory):

| layout | lj | orkut |
|---|---|---|
| hcl | 0.63 | 0.73 |
| hcls | 1.00 | 1.00 |
| hclsb | 1.15 | 1.11 |
| cells | 2.92 | 2.36 |
| cellsb | 3.45 | 3.29 |
| cellsd | 5.41 | 3.27 |

Tiles (`tiles/<variant>_<graph>.json`; `gmap tiles` at 8 threads, zmax chosen by the tiles stage from the node radii):

| graph | layout | zmax | tiles | file MB |
|---|---|---|---|---|
| lj | hcls | 10 | 236,602 | 988 |
| lj | cells | 7 | 20,922 | 315 |
| lj | cellsd | 10 | 362,705 | 1300 |
| orkut | hcls | 10 | 177,440 | 923 |
| orkut | cells | 7 | 21,265 | 272 |
| orkut | cellsd | 9 | 96,033 | 605 |

At 8 threads, peak RSS is 0.83 GB (lj) and 1.24 GB (orkut) for the disc layouts, and
1.03–1.24 GB and 1.36–1.44 GB with cells. The cells code is a direct lab version:
O(k² · vertices) clipping per Lloyd step, a 24-step bisection per inscribed circle, and
48-gon parents in `cellsd`. It was not optimized; 5 Lloyd steps instead of 20 gave 1.8% mean
misallocation on lj and node discs on 62% of the map (`layout/cells5_lj.json`, one run).

## Recommendation for m5-01 (#67)

- **Build `hcls`**: nested discs, siblings placed by stress on the parent's quotient graph,
  then packed (ADR 0017 as amended). It is the best of the tested layouts on the local and
  community measures, ties or leads on the global ones, and costs 1.4–1.6× `hcl` at 1 thread.
- **Drop the "pull towards external neighbours" term.** `hcls` has none. Its redesign,
  boundary-aware placement (`hclsb`), lowers NP₁ and NP@10 and does not shorten edges between
  top regions, at 1.1–1.2× the time.
- **Do not replace disc packing with cells (no-go for m5-01).** Cells use 39–40× more of
  the map for nodes, but they lose on the local and community measures that are the
  layout's strength, cost 2.4–2.9× the time, and do not fit the approved bubble style
  (their nodes leave the drawn bubbles). The lab code (`src/cells.rs`) stays for a possible
  "cells" view, which `design/DECISIONS.md` does not require for 0.1.
- **Keep ADR 0017's narrowed claim** (distance between large regions carries little
  meaning) and its re-decision trigger. The global gap is set by which top regions are
  neighbours and by compact regions, not by member placement; the viewer must show
  inter-region connectivity, because geometry alone does not.
- **The area trigger of ADR 0017 is met (≥ 10×) but its other condition is not** ("the same
  cost and determinism", and m5-01's "none worse than `hcls`"). The way to more area inside
  the bubble style is denser packing at each depth (`cellsd` shows 0.73 per depth is
  possible with nesting). That is a tuning question for m5-01's packing, not a reason to
  change D17.

## What was not measured

- Disc packing with compaction passes and sibling ordering by quotient affinity (an issue
  candidate), and a larger φ. Either would raise the area of nested discs; neither was run.
- Parallelizing the serial apply pass: it is in m5-01's scope; no measurement here.
- A placement of the top regions that puts linked big regions next to each other (the
  remaining lever on orkut), and challenge 4's space-filling curves for giant communities.
- DRGraph was not re-run; its rows are m1-18's files, scored by the same scripts.
- Any time on an idle machine.

## Reproduce

`../run_all.sh` runs the steps build, layouts, determinism, metrics, diagnostics and tiles; scratch
directory `/home/tux/graphlab/scratch/m1-07-layout-area`. `../table.py` prints the tables.
