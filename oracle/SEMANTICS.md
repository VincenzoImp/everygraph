# Semantics table: where the reference libraries disagree, and what Everygraph does

The correctness oracle compares Everygraph with NetworkX 3.7, python-igraph 1.0.0 and
scipy 1.18.1. On a simple undirected graph they agree with each other (0 disagreements on
the 301 graphs of the CI corpus, `oracle-ref run`). They part ways on self-loops,
repeated pairs, direction and degenerate inputs. This table records each case, what each
library does, and what Everygraph does and why.

**Evidence.** Every value below is in [`semantics/probe.json`](semantics/probe.json),
written by `uv run oracle-ref probe --write`. The probe gives each library the raw graph
(a `MultiGraph`/`MultiDiGraph` in NetworkX, a multigraph in igraph), so their own
conventions show. The Everygraph column is the oracle's reference answer on the analytic
graph, the answer every Everygraph kernel is checked against. `uv run oracle-ref probe
--check` fails if a library upgrade changes any value (reals rounded to 6 decimals).

**Where the rules come from.** `plan/issues/m2-09-graph-semantics.md` and the approved
defaults in `design/preview/choices.md`:

- **A1.** Repeated pairs are kept as events and analysed once per pair. The pair's
  multiplicity and aggregated weight are available to weighted kernels.
- **A2.** Community detection, modularity included, ignores direction.
- **A3.** Self-loops are dropped and counted, unless the caller keeps them
  (`self_loops = "keep"`).
- **A4.** Direction is given by the caller.
- **`duplicates`** (`docs/spec/graph-semantics.md` § 2.3–2.4). Loops are treated
  first. `events` (the default, A1) stores every row; `collapse` drops a repeated row and
  keeps the pair's first row in input order (multiplicity 1, its weight); `sum` merges the
  rows into one edge, which analyses exactly as `events` except that a path's step costs
  the stored sum. The ingest report (`ingest_report`) adds up:
  `rows = rejected + dropped.self_loops + dropped.duplicates + merged + edges` and
  `edges = pairs + repeated`.

Rows marked with a question number (Q1–Q13) are choices those sources did not dictate.
They were decided on 2026-11-25 and recorded in `design/DECISIONS.md`, row "Graph
semantics, as the correctness oracle checks them"; see "Decisions" at the end. All were
decided as the oracle first proposed them except Q10. Q2 was decided by #36
(`docs/spec/graph-semantics.md` § 5; `design/DECISIONS.md`, row "Graph semantics, the
cases left to #36"), whose rule S1–S6 the rows below cite.

## Self-loops (graph: triangle 0-1-2 plus a loop at 0, loops kept)

One principle (S): a kept loop is the pair {v, v} of weight w, `A[v][v] = 2w` on an
undirected graph and `w` on a directed one. Degrees, PageRank, modularity and the quotient
count it through that matrix; core numbers, triangles, clustering and transitivity ignore
it.

| Case | NetworkX | igraph | Everygraph | Rule |
|---|---|---|---|---|
| degree | [4, 2, 2] | [4, 2, 2] | [4, 2, 2] | **decided (Q1)**: a kept loop adds 2 to the undirected degree, and 1 each to the out-degree and in-degree, as in both libraries. Repeated loop rows count as one loop, like any repeated pair (A1) |
| degree, default | no option | no option | [2, 2, 2] | A3: loops dropped |
| core numbers | refuses (`NetworkXNotImplemented`) | [2, 2, 2] (counts the loop) | [2, 2, 2] | **S2**: loops ignored, as NetworkX on the loop-free graph |
| core numbers, a lone node with a loop (node 1 isolated) | refuses | [2, 0] | [0, 0] | **S2**: igraph counts the loop's 2 in the degree |
| core numbers, edge 0-1 with a loop at each end | refuses | [3, 3] | [1, 1] | **S2** |
| PageRank, undirected | [0.416, 0.292, 0.292] | [0.475, 0.2625, 0.2625] | [0.475, 0.2625, 0.2625] | **S1**, as igraph: `A[v][v] = 2w`. NetworkX puts `w` on the diagonal; the oracle gives it the loop with weight 2w |
| PageRank, weighted, loop of weight 3 | [0.519, 0.240, 0.240] | [38/63, 25/126, 25/126] | [38/63, 25/126, 25/126] | **S1**, as igraph (and NetworkX given weight 6) |
| PageRank, directed (3-cycle plus a loop at 0) | [0.480, 0.254, 0.266] | same | same | **S1**: `A[v][v] = w`, both libraries agree |
| triangles | [1, 1, 1] | [1, 1, 1] | [1, 1, 1] | all three ignore loops |
| clustering | [1, 1, 1] | [1, 1, 1] | [1, 1, 1] | **S3**: k′ = neighbours other than v, as both libraries |
| clustering, with a pendant 2-3 | [1, 1, 0.333, 0] | same | same | **S3** |
| transitivity | 1 | 1 | 1 | **S4** |
| transitivity, with a pendant 2-3 | 0.6 | 0.6 | 0.6 | **S4** |
| modularity, partition {0,1} {2} | −0.125 | −0.125 | −0.125 | **S5**: the loop in its community's internal pairs once and 2w in the strength, as both libraries |
| modularity, weighted, loop of weight 3 | −1/18 | −1/18 | −1/18 | **S5** |
| modularity, directed (3-cycle plus a loop at 0) | −0.125 (directed formula) | −0.125 | −0.125 | **S5** on the undirected projection (A2) |

## Repeated pairs (graph: triangle 0-1-2, pendant 2-3, pair 0-1 given twice)

| Case | NetworkX | igraph | Everygraph | Rule |
|---|---|---|---|---|
| degree | [3, 3, 3, 1] | [3, 3, 3, 1] | [2, 2, 3, 1] | A1: distinct neighbours. The number of events is the weighted degree with weight = count (m3-07) |
| core numbers | refuses | [2, 2, 2, 1] | [2, 2, 2, 1] | A1 |
| triangles | [1, 1, 1, 0] | [1, 1, 1, 0] | [1, 1, 1, 0] | agree |
| clustering | refuses | [1, 1, 0.333, 0] | [1, 1, 0.333, 0] | A1 (igraph ignores the multiplicity too) |
| transitivity | refuses | 0.6 | 0.6 | A1 |
| PageRank | [0.2857, 0.2857, 0.3047, 0.1238] (multiplicity as weight) | same | [0.2459, 0.2459, 0.3667, 0.1414] | A1: unweighted on the pairs by default |
| PageRank, `weighted = true` | as above | as above | [0.2857, 0.2857, 0.3047, 0.1238] | **decided (Q8)**: weighted PageRank uses the *sum* of the event weights of a pair (the multiplicity when there is no weight column), which reproduces both libraries |
| modularity, partition {0,1} {2,3} | 0.08 | 0.08 | 0.0 | A1 and A2: unweighted, on the pairs |
| weighted distance from 0, weights 5 and 1 on the two 0-1 rows | [0, 1, 1, 2] | [0, 1, 1, 2] | [0, 1, 1, 2] | **decided (Q8)**: shortest paths use the *smallest* event weight of a pair, as both libraries do |

## Direction (graph: 3-cycle 0→1→2→0, plus arcs 0→2 and 3→0)

| Case | NetworkX | igraph | Everygraph | Rule |
|---|---|---|---|---|
| degree | [4, 2, 3, 1] (in + out) | [4, 2, 3, 1] (in + out) | [3, 2, 2, 1] | **decided (Q10)**: distinct neighbours in either direction, the degree in the undirected projection; a reciprocal pair is one neighbour. Both libraries add in- and out-degree. `degree_out` and `degree_in` count distinct out- and in-neighbours |
| core numbers | [2, 2, 2, 1] | [2, 2, 2, 1] | [2, 2, 2, 1] | — |
| core numbers, arcs 0→1 and 1→0 only | [2, 2] | [2, 2] | [1, 1] | **decided (Q3)**: on the undirected projection, where a reciprocal pair is one edge. The libraries use in + out, so a reciprocal pair alone makes a 2-core |
| triangles | refuses | [1, 1, 1, 0] | [1, 1, 1, 0] | **decided (Q4)**: on the undirected projection, as igraph |
| clustering | [0.2, 1, 0.5, 0] (Fagiolo's directed clustering) | [0.333, 1, 1, 0] | [0.333, 1, 1, 0] | **decided (Q4)**: undirected projection, as igraph; directed clustering is not offered |
| transitivity | 0.5 | 0.6 | 0.6 | **decided (Q4)**: undirected projection, as igraph |
| modularity, two directed 3-cycles joined by 0→3 | 0.3673 (directed formula) | 0.3673 (directed formula) | 0.3571 | A2: direction ignored |
| BFS from 2 on 0→1→2 | [–, –, 0] | [∞, ∞, 0] | [–, –, 0] | out-edges; "unreachable" is `null` in the exchange form (Q12) |
| PageRank with dangling nodes (0→1→2, node 3 isolated) | [0.1557, 0.2880, 0.4005, 0.1557] | same | same | agree: dangling mass spread uniformly |

## Degenerate inputs

| Case | NetworkX | igraph | Everygraph | Rule |
|---|---|---|---|---|
| clustering where degree < 2 (one edge plus an isolate) | 0 | NaN (default mode) | 0 | **decided (Q5)**: 0, as NetworkX and as igraph's `mode="zero"` |
| transitivity without any connected triple | 0 | NaN | 0 | **decided (Q6)**: 0, as NetworkX |
| modularity of a graph without edges | 0 | NaN | 0 | **decided (Q7)**: 0, as NetworkX |
| PageRank of the graph with no nodes | empty | empty | empty | agree |

## PageRank convergence

NetworkX stops when the L1 change between two iterates is below `n · tol` (default
`tol = 1e-6`), so its default answer gets less accurate as the graph grows. For n > 2·10⁶
the bound exceeds 2, the largest possible L1 change, and NetworkX stops after one
iteration (`m3-04`). On a random directed graph with 3,000 nodes, the default answer is up
to 8·10⁻⁶ away from igraph's PRPACK solution. With `tol = 1e-15` the gap is about 8·10⁻¹⁵ (the probe records it as below 10⁻¹³, since the last digit varies between runs)
(`pagerank-convergence` in the probe). igraph solves the linear system directly.

A fixed `tol` is not enough either. On the nightly corpus's power-law graph with 20,000
nodes, NetworkX at `tol = 1e-15` was 2.5·10⁻⁸ relative away from PRPACK on some nodes:
the stopping bound n·tol grows with n. NetworkX and igraph disagreed there by more than
the PageRank tolerance, and the in-process reference sided with igraph. At
`tol = 1e-14 / n` the gap is 1.7·10⁻¹¹.

The oracle compares every PageRank against the **fixed point**. NetworkX runs with
`tol = 1e-14 / n` and `max_iter = 20000` (`ADJUSTMENTS` in `refs.py`), igraph with PRPACK,
and the in-process reference iterates until the initial error has shrunk by 10⁻¹⁷. That
takes 241 iterations at damping 0.85. An Everygraph kernel may stop by any rule, as long
as it lands within the PageRank tolerance (`tolerances.json`, `oracle/README.md`). The
NetworkX backend (D20) must reproduce NetworkX's stopping rule; the native API must not
(Q13).

## Adjustments made to the libraries' answers

The runners map these library conventions to Everygraph's before comparing. Each one is
listed in `ADJUSTMENTS` in `src/everygraph_oracle_ref/refs.py` and copied into every
`reference.json`:

- an undirected kept loop is given to NetworkX's PageRank with weight 2w (2 when
  unweighted), so that it sees `A[v][v] = 2w` (S1); igraph doubles the loop itself;
- loops are removed before core numbers (both libraries), clustering and transitivity:
  igraph counts them in core numbers and NetworkX refuses them (S2);
- igraph's local transitivity is called with `mode="zero"`;
- igraph's NaN transitivity and NaN modularity become 0;
- NetworkX's PageRank runs with `tol = 1e-14 / n`, retried at `1e-12 / n` and `1e-10 / n` where
  rounding noise keeps it from converging (seen on the star with 2^16 leaves); the comparison
  with igraph and the in-process reference then decides whether that answer is close enough
  within `max_iter`.

## Kept self-loops and `duplicates`: what is checked

Every kernel, on every graph, under each `duplicates` mode. With kept loops the adjacency
lists a looped node once in its own list; the quotient graph carries kept loops on its
diagonal (S6), so its weights sum to m with the loops. The invariants that cover the
decided cases: `self_loop_exactly_where_kept`, `pagerank_sums_to_one` and
`pagerank_fixed_point` (S1), `modularity_equals_quotient_modularity` (S5, S6),
`report_rows_add_up` and `report_edges_add_up`; property tests check that core numbers,
clustering and transitivity are the same with loops kept and dropped, and that `sum`
analyses as `events`.

## Decisions (2026-11-25, `design/DECISIONS.md`)

1. **Q1. Kept self-loops in degrees.** +2 to the undirected degree (and to `degree` of a
   directed graph), +1 to out- and in-degree. Repeated loop rows are one loop.
2. **Q2. PageRank and core numbers with kept self-loops.** Decided by #36
   (docs/spec/graph-semantics.md § 5): PageRank and modularity count a kept loop as
   A[v][v] = 2w undirected, w directed; core numbers, clustering and transitivity ignore
   it; the quotient carries it on its diagonal.
3. **Q3. Core numbers of a directed graph.** On the undirected projection, where a
   reciprocal pair is one edge.
4. **Q4. Triangles, clustering and transitivity of a directed graph.** On the undirected
   projection, as igraph.
5. **Q5, Q6, Q7. Undefined ratios.** Clustering at degree < 2, transitivity without a
   connected triple and modularity without edges are 0, never NaN.
6. **Q8. Weights of a repeated pair.** PageRank sums them; shortest paths take the minimum.
7. **Q9. Undirected projection of a directed graph.** It sums the events of both
   directions.
8. **Q10. `degree` of a directed graph.** Distinct neighbours in either direction (the
   degree in the undirected projection), *not* in + out as NetworkX and igraph define it.
   The oracle had proposed in + out; the decision changed it. `degree_out` and
   `degree_in` count distinct out- and in-neighbours.
9. **Q11. Component labels.** A component's internal label is its earliest node (with
   dense ids, its smallest node id; the API then numbers components by size).
10. **Q12. Traversals.** They follow out-edges unless `direction=` says otherwise; an
    unreachable node has no value.
11. **Q13. PageRank.** The fixed point, whatever the iteration count.

The reason recorded with the decision: one projection for everything that ignores
direction, so that the numbers a user sees on one graph agree with each other; where the
libraries disagree with each other, follow the one that returns a number.
