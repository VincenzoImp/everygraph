# Semantics table: where the reference libraries disagree, and what Everygraph does

The correctness oracle compares Everygraph with NetworkX 3.7, python-igraph 1.0.0 and
scipy 1.18.1. On a simple undirected graph they agree with each other (0 disagreements on
the 288 graphs of the CI corpus, `oracle-ref run`). They part ways on self-loops,
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
- **A3.** Self-loops are dropped and counted.
- **A4.** Direction is given by the caller.

Rows marked **open** are choices these sources do not dictate. I made them for the
oracle, and the coordinator must confirm or change them; see "Open questions" below.

## Self-loops (graph: triangle 0-1-2 plus a loop at 0)

| Case | NetworkX | igraph | Everygraph | Rule |
|---|---|---|---|---|
| degree, loops kept | [4, 2, 2] | [4, 2, 2] | [4, 2, 2] | **open (Q1)**: a kept loop adds 2 to the undirected degree, and 1 each to the out-degree and in-degree, as in both libraries. Repeated loop rows count as one loop, like any repeated pair (A1) |
| degree, default | no option | no option | [2, 2, 2] | A3: loops dropped |
| core numbers | refuses (`NetworkXNotImplemented`) | [2, 2, 2] (counts the loop) | [2, 2, 2] | A3; with loops kept the oracle defines no answer (Q2) |
| PageRank, undirected | [0.416, 0.292, 0.292] | [0.475, 0.263, 0.263] | [1/3, 1/3, 1/3] | A3. **The libraries disagree with each other**: NetworkX puts a weight of 1 on the loop's matrix entry, igraph counts the loop twice in the degree. With loops kept, the oracle defines no answer (Q2) |
| PageRank, directed (3-cycle plus a loop at 0) | [0.480, 0.254, 0.266] | same | [1/3, 1/3, 1/3] | A3 |
| triangles | [1, 1, 1] | [1, 1, 1] | [1, 1, 1] | all three ignore loops |
| clustering | [1, 1, 1] | [1, 1, 1] | [1, 1, 1] | all three ignore loops |
| modularity, partition {0,1} {2} | −0.125 | −0.125 | −0.2222 | A3: the libraries count the loop in the degrees and in the internal edges |

## Repeated pairs (graph: triangle 0-1-2, pendant 2-3, pair 0-1 given twice)

| Case | NetworkX | igraph | Everygraph | Rule |
|---|---|---|---|---|
| degree | [3, 3, 3, 1] | [3, 3, 3, 1] | [2, 2, 3, 1] | A1: distinct neighbours. The number of events is the weighted degree with weight = count (m3-07) |
| core numbers | refuses | [2, 2, 2, 1] | [2, 2, 2, 1] | A1 |
| triangles | [1, 1, 1, 0] | [1, 1, 1, 0] | [1, 1, 1, 0] | agree |
| clustering | refuses | [1, 1, 0.333, 0] | [1, 1, 0.333, 0] | A1 (igraph ignores the multiplicity too) |
| transitivity | refuses | 0.6 | 0.6 | A1 |
| PageRank | [0.2857, 0.2857, 0.3047, 0.1238] (multiplicity as weight) | same | [0.2459, 0.2459, 0.3667, 0.1414] | A1: unweighted on the pairs by default |
| PageRank, `weighted = true` | as above | as above | [0.2857, 0.2857, 0.3047, 0.1238] | **open (Q8)**: weighted PageRank uses the *sum* of the event weights of a pair (the multiplicity when there is no weight column), which reproduces both libraries |
| modularity, partition {0,1} {2,3} | 0.08 | 0.08 | 0.0 | A1 and A2: unweighted, on the pairs |
| weighted distance from 0, weights 5 and 1 on the two 0-1 rows | [0, 1, 1, 2] | [0, 1, 1, 2] | [0, 1, 1, 2] | **open (Q8)**: shortest paths use the *smallest* event weight of a pair, as both libraries do |

## Direction (graph: 3-cycle 0→1→2→0, plus arcs 0→2 and 3→0)

| Case | NetworkX | igraph | Everygraph | Rule |
|---|---|---|---|---|
| degree | [4, 2, 3, 1] | [4, 2, 3, 1] | [4, 2, 3, 1] | **open (Q10)**: out + in, with the two arcs of a reciprocal pair counted separately, as in both libraries. Out-degree and in-degree are their own kernels |
| core numbers | [2, 2, 2, 1] | [2, 2, 2, 1] | [2, 2, 2, 1] | — |
| core numbers, arcs 0→1 and 1→0 only | [2, 2] | [2, 2] | [1, 1] | **open (Q3)**: on the undirected projection, where a reciprocal pair is one edge. The libraries use in + out, so a reciprocal pair alone makes a 2-core |
| triangles | refuses | [1, 1, 1, 0] | [1, 1, 1, 0] | **open (Q4)**: on the undirected projection, as igraph |
| clustering | [0.2, 1, 0.5, 0] (Fagiolo's directed clustering) | [0.333, 1, 1, 0] | [0.333, 1, 1, 0] | **open (Q4)**: undirected projection, as igraph; directed clustering is not offered |
| transitivity | 0.5 | 0.6 | 0.6 | **open (Q4)**: undirected projection, as igraph |
| modularity, two directed 3-cycles joined by 0→3 | 0.3673 (directed formula) | 0.3673 (directed formula) | 0.3571 | A2: direction ignored |
| BFS from 2 on 0→1→2 | [–, –, 0] | [∞, ∞, 0] | [–, –, 0] | out-edges; "unreachable" is `null` in the exchange form (Q12) |
| PageRank with dangling nodes (0→1→2, node 3 isolated) | [0.1557, 0.2880, 0.4005, 0.1557] | same | same | agree: dangling mass spread uniformly |

## Degenerate inputs

| Case | NetworkX | igraph | Everygraph | Rule |
|---|---|---|---|---|
| clustering where degree < 2 (one edge plus an isolate) | 0 | NaN (default mode) | 0 | **open (Q5)**: 0, as NetworkX and as igraph's `mode="zero"` |
| transitivity without any connected triple | 0 | NaN | 0 | **open (Q6)**: 0, as NetworkX |
| modularity of a graph without edges | 0 | NaN | 0 | **open (Q7)**: 0, as NetworkX |
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
`tol = 1e-14 / n` and `max_iter = 100000` (`ADJUSTMENTS` in `refs.py`), igraph with PRPACK,
and the in-process reference iterates until the initial error has shrunk by 10⁻¹⁷. That
takes 241 iterations at damping 0.85. An Everygraph kernel may stop by any rule, as long
as it lands within the PageRank tolerance (`tolerances.json`, `oracle/README.md`). The
NetworkX backend (D20) must reproduce NetworkX's stopping rule; the native API must not
(Q13).

## Adjustments made to the libraries' answers

The runners map these library conventions to Everygraph's before comparing. Each one is
listed in `ADJUSTMENTS` in `src/everygraph_oracle_ref/refs.py` and copied into every
`reference.json`:

- igraph's local transitivity is called with `mode="zero"`;
- igraph's NaN transitivity and NaN modularity become 0;
- NetworkX's PageRank runs with `tol = 1e-14 / n`, retried at `1e-13 / n` if it fails to converge
  within `max_iter`.

## Open questions for the coordinator

Each one is a choice I made for the oracle; it fixes what every kernel will be tested
against.

1. **Q1. Kept self-loops in degrees.** +2 to the undirected degree, +1 to out and in, and
   repeated loop rows are one loop. Follows both libraries, except for the collapse of
   repeated loops, which follows A1.
2. **Q2. PageRank and core numbers with kept self-loops.** Left undefined: the oracle does
   not check these kernels when loops are kept. NetworkX and igraph disagree on PageRank,
   and NetworkX refuses core numbers. This has to be decided when the "keep self-loops"
   option is built (m2-09).
3. **Q3. Core numbers of a directed graph.** Computed on the undirected projection, where
   a reciprocal pair is one edge. Both libraries use in + out and count it twice. m3-07
   does not say; the k-core is an undirected notion.
4. **Q4. Triangles, clustering and transitivity of a directed graph.** Undirected
   projection, as igraph does. NetworkX refuses triangles and computes Fagiolo's directed
   clustering.
5. **Q5, Q6, Q7. Undefined ratios.** Clustering at degree < 2, transitivity without a
   connected triple, and modularity without edges are all 0, as in NetworkX (the
   NetworkX backend must match it anyway). The alternative is NaN, as in igraph.
6. **Q8. Aggregating weights of a repeated pair.** PageRank sums them, shortest paths take
   the minimum. Both libraries behave this way on multigraphs.
7. **Q9. Undirected projection of a directed graph.** It aggregates the events of both
   directions: the multiplicity of {u, v} is the number of arcs either way. No kernel
   reads that weight today.
8. **Q10. "Degree" of a directed graph.** out + in, with a reciprocal pair counted twice.
9. **Q11. Component labels.** The smallest node id of the component, for weak components
   (m3-05) and, by extension, strong ones.
10. **Q12. Traversals of directed graphs.** They follow out-edges; an unreachable node
    has no value.
11. **Q13. PageRank.** The answer is the fixed point, never a "stopped after N·tol"
    approximation.
