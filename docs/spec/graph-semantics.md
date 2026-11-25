# Graph semantics: direction, self-loops, repeated edges, weights, time

Status: specification for 0.1, written by #36 (`plan/issues/m2-09-graph-semantics.md`);
§ 2.3, § 2.4, § 2.5, § 2.7 and § 4.3 amended on 2026-11-25 by ADR 0028 (pairs in the
adjacency, events beside it; rules for files built by appends).
Every reader (#32, #33), the build (#35), the property columns (#37), the kernels (#44–#51),
community detection (#57, #58, #62), the point queries (#90, #91) and the views (#93, #94)
implement what is written here, and the correctness oracle (#25, `oracle/`) checks it.

**Authority.** The rows of `design/DECISIONS.md` (2026-11-25: "Graph semantics, as the
correctness oracle checks them", "Default weight for PageRank and communities", "Time
windows", "Nodes in a view", "Degree columns", "Identifiers in results") and the approved
defaults of `design/preview/choices.md` A1–A4 come first. This document applies them,
settles what they leave open (§ 5, § 10) and is consistent with `docs/api-sketch.md`
(§ 2.2, 2.3, 2.4, 2.12, 2.13, 3.1) and `docs/ux-contract.md` § 3.6. Where those two
disagreed with the decision records, the decision records won and the disagreement is
listed in § 9.

Contents: 1 Terms · 2 Ingest · 3 The analytic graph · 4 Weights · 5 Self-loops in the
kernels (the cases #36 decides) · 6 What each operation does on each kind of graph · 7
Refusals · 8 What the oracle checks · 9 Disagreements found · 10 Decided here.

---

## 1. Terms

| Term | Meaning |
|---|---|
| **row** | One record of the input: a source, a target and optional columns. |
| **event** | A row that became a stored edge. `g.num_edges` counts events. |
| **pair** | On a directed graph, an ordered pair `(u, v)`; on an undirected graph, an unordered pair `{u, v}` (the rows `a,b` and `b,a` are the same pair). A **loop** is the pair of a node with itself. |
| **multiplicity** `mult(p)` | The number of events of pair `p`. |
| **analytic graph** | What the kernels see: one entry per pair, with its multiplicity and aggregated columns (§ 3). Every kernel runs on it; none sees events one by one except `num_edges`, `edges()`, the event counts of `c.table()` and the time filters. |
| **undirected projection** | Of a directed graph: the pair `{u, v}` is present when `(u, v)` or `(v, u)` is, and aggregates the events of both directions (DECISIONS, Q9). Of an undirected graph: the graph itself. |
| **kept loop** | A loop of a graph read with `self_loops="keep"`. Repeated loop rows of one node are one pair, like any repeated pair (Q1). |
| **N(v)** | The neighbours of `v` in the analytic graph; it contains `v` itself exactly when `v` has a kept loop. **N'(v)** = N(v) \ {v}. On a directed graph N⁺(v) and N⁻(v) are the out- and in-neighbours, N(v) the neighbours in the undirected projection. |

## 2. Ingest

### 2.1 The options

The public options are those of `docs/api-sketch.md` § 3.1 (`eg.read`, `eg.from_edges`)
and § 5.2 (`everygraph ingest`). The values the plan and the summary print are the
enumerations of `docs/ux-contract.md` § 3.6 (`detected.edges`).

<!-- check: options -->
| Option | Values | Default | Printed in the plan's `edges` line and in `detected.edges` |
|---|---|---|---|
| `directed=` / `--directed`, `--undirected` | `True`, `False`, `None` | `None`: inferred (§ 2.2) | `directed`, `undirected`; `directed_from`: `caller`, `column names`, `default` |
| `self_loops=` / `--self-loops` | `"drop"`, `"keep"` | `"drop"` (A3) | `self-loops dropped`, `self-loops kept` — values `dropped`, `kept` |
| `duplicates=` / `--duplicates` | `"events"`, `"collapse"`, `"sum"` | `"events"` (A1) | `duplicates kept as events`, `duplicates collapsed`, `duplicates summed` — values `kept as events`, `collapsed`, `summed` |
<!-- /check: options -->

`weight=` and `time=` name the weight and time columns (§ 2.6). The options are part of
the graph's identity: the same input read with other options is another graph (the cache
key of `docs/api-sketch.md` § 2.7).

### 2.2 Direction (A4)

Given by the caller. When `directed=None`, the graph is directed if the source and target
columns are named like a direction — `from`/`to` or `source`/`target`, compared without
regard to case — and undirected otherwise; the summary says which and why
(`directed (from the column names)`, `undirected (the column names give no direction)`).
`eg.from_edges` defaults to `directed=False` and `eg.from_scipy` to "directed unless the
matrix is symmetric", as `docs/api-sketch.md` § 3.1 says.

### 2.3 Order of the treatments, and the node set

Each row goes through these steps, in this order, and lands in exactly one outcome:

1. **Read.** A row that cannot be read (wrong number of fields, a value not of its
   column's type) stops the ingest with `InvalidInput` (exit status 3). There is no
   option to skip invalid rows in 0.1 (`docs/ux-contract.md` § 10, #32), so `rejected`
   is always 0 in a finished ingest; the field exists so the report keeps its shape when
   that option arrives.
2. **Self-loops.** A row whose source equals its target is a loop row. With
   `self_loops="drop"` it is dropped and counted; with `"keep"` it goes on to step 3
   like any row. Loops are treated before duplicates: a dropped loop row is never also
   counted as a duplicate.
3. **Duplicates.** A row whose pair (§ 1) was already seen in an earlier row is a
   repeated row. `"events"`: stored as its own event. `"collapse"`: dropped and counted;
   the pair keeps its first row in input order. `"sum"`: merged into the pair's one
   edge (§ 2.4) and counted.
4. **Stored** as an event.

**Input order** is the order of the rows in the file, files in the order given, row
groups of a Parquet file in file order. It decides which row `collapse` keeps and breaks
ties in the storage order (§ 2.5); it never depends on threads or the budget.

**Nodes.** Every identifier that appears in a row that was read (steps 2–4) is a node,
including one whose only rows were dropped loops: it is then an isolate, counted in the
report. *The node set does not change with an option that is about edges, and no
identifier the user gave disappears without being counted.* Graph order (the order of
`g.nodes()`, `docs/api-sketch.md` § 2.2) is #34's.

**Graph order after appends** (segments, #111; not in 0.1). In a file built by appends,
graph order is the order of the base followed, for each segment in turn, by its new
identifiers in the order that segment's input first names them. A fresh ingest of the
concatenated inputs gives the same order only when ids are assigned by first appearance:
the input of an appended file is its sequence of batches (ADR 0028).

### 2.4 What each `duplicates` value stores

| | `"events"` (default) | `"collapse"` | `"sum"` |
|---|---|---|---|
| Edges stored per pair | one per row | one | one |
| The weight column | each row's value | the first row's value | the exact sum of the rows' values (§ 4.3), under the weight column's name |
| The time column | each row's time | the first row's time | replaced by `first` and `last`: the earliest and latest non-null time of the rows (null when all are null) |
| `edges` (uint64) | — (the multiplicity is counted from the events) | — (multiplicity 1) | the number of rows merged into the edge, ≥ 1 |
| `num_edges` | rows stored | pairs | pairs |
| What the analytic graph sees | multiplicity and aggregates of the events | multiplicity 1, the first row's values | multiplicity = `edges`, weight = the stored sum, first/last time |

Under every option the adjacency holds **one entry per pair**; under `"collapse"` and
`"sum"` each pair has one event, so there is no `.events` section (§ 2.5) and every edge
column is per pair.

**Consequence, by design:** a graph read with `"sum"` gives the same result as the same
input read with `"events"` for every operation that works on pairs — degrees, PageRank,
communities, components, core numbers, triangles, `neighbors()` — because the analytic
graph is the same. What is lost is what needs the events: time windows (refused, § 7),
the smallest event weight (a weighted path uses the sum, § 6), and the event counts
(`num_edges`, `internal_edges`, which count stored edges). `"collapse"` changes results
wherever multiplicity or a weight enters (it is the "smaller file, history gone" of A1).

Minimum and maximum of a weight per pair are not offered in 0.1 (`docs/api-sketch.md`
§ 7); the issue's "count, sum/min/max, first/last" is reduced to `sum`.

### 2.5 The event model and storage order

The adjacency stores one entry per pair, in increasing internal id of the neighbour —
which is graph order until a renumbering (`plan/issues/m2-07-id-mapping.md`,
`m2-24-bp-ordering.md`); the events of a pair are stored, in **(time, input order)** order
with nulls of time last, in the event columns, at the event offsets the adjacency's
`.events` section gives (`docs/spec/eg-adjacency.md` § 3.1). For an undirected graph each
pair appears in the lists of both ends, with the same events in the same order at both; a
loop appears once, in its node's list. The events of a pair are therefore contiguous, and
the analytic graph is the adjacency itself — the pair's neighbour and its multiplicity:
the stored events `ev[e + 1] − ev[e]`, 1 when there is no `.events` section, except under
`"sum"`, where the multiplicity is the `edges` column (§ 2.4), so that `"sum"` and
`"events"` give the same pair results — with aggregates over each pair's events (§ 4)
computed by the build from the event range and stored as per-pair columns. Kernels read
pairs, multiplicities and per-pair columns, never per-event columns; time windows,
`edges()` and event counts read the events through the event offsets. Which aggregates
are stored per pair and which are computed from the events when asked is #37's; the pair
sum of a weight column (§ 4.3), which every weighted kernel reads, is stored. On a
directed graph the in-adjacency (#38) is ordered the same way and has its own `.events`
section. Storage order is fixed by the input and the options, never by threads, budget or
regime (`docs/api-sketch.md` § 2.11).

Results that list nodes or neighbours "in graph order" (`docs/api-sketch.md` § 2.12:
`neighbors()`, the position in a parent's list that orders `khop()`, the `bfs()` parent)
are ordered by graph position, not by storage order; the two differ only after a
renumbering.

### 2.6 Column types

| Role | Accepted Arrow types | Stored as | Notes |
|---|---|---|---|
| weight | `float32`, `float64`; `int8`–`int64`, `uint8`–`uint64`; `decimal128(p, s)`, `decimal256(p, s)` | the input type, exactly | Integers and decimals are kept exact — token amounts do not fit in `float32`, nor 18-decimal amounts in `float64` (`docs/api-sketch.md` § 3.4: "a weight that was a decimal or an integer in the input stays exact"). Text in a weight column is `InvalidInput` (3). `decimal256` needs adding to #37's list of column types (§ 9). |
| time | Arrow `timestamp` (any unit; a zone is converted to UTC, a naive time is UTC), `date32`/`date64` (stored as the day's first instant), any integer type (compared in its own unit: block number, epoch count) | timestamp in UTC, or the integer type | Text is parsed by the readers (#32, #33) as ISO 8601 or refused with `InvalidInput`. Windows and filters on it: `docs/api-sketch.md` § 2.3 ("Time windows"). |
| edge type | a dictionary-encoded or text column with few distinct values | dictionary-encoded, `uint8` codes up to 256 values, `uint16` up to 65,536, otherwise a plain text column | A small categorical column, filterable (`g.filter(edges="kind == 'swap'")`). No kernel treats it specially in 0.1. **Gap:** `eg.read` reads only `source`, `target`, `weight` and `time` and lists the other columns as ignored; reading a further column needs a public option, which needs a preview (§ 9). |

**Nulls.** A null weight is skipped in sums (a pair whose weights are all null weighs 0)
and never used as a path cost; a null time is never inside a window (a comparison with a
null is false) and is skipped by `first` and `last`. A null source or target is a row
that cannot be read (`InvalidInput`).

### 2.7 Column statistics, recorded by the build

For every numeric edge column (weight, time and any other) the build records, over the
stored values (after `sum` where it applies), in the file's table of contents (#28, #39):

| Statistic | Type | Use |
|---|---|---|
| `count` | uint64 | non-null values |
| `null_count` | uint64 | nulls |
| `nan_count` | uint64 | NaN values (0 for exact types) |
| `min`, `max` | the column's type, null when there is no non-NaN value | over the non-null, non-NaN values; ±∞ shows up here |
| `sum` | float64 | the exact sum rounded once (§ 4.3), over the non-null, non-NaN values |

and, per graph, the largest multiplicity of a pair.

**Files built by appends** (segments, #111; not in 0.1). Column statistics are recorded
per run and for the whole graph. The whole graph's are kept exact and updated
incrementally, from what each append writes, never by a rescan of older runs:

- `count`, `null_count`, `nan_count`, and `min` and `max` of a per-event column: combined
  from the run's values;
- `sum`: each run records its exact accumulator (not its `float64` rounding), and the whole
  graph's exact sum is the exact sum of those, rounded once (§ 4.3);
- the largest multiplicity: the larger of the recorded one and the multiplicities, over
  all runs, of the pairs the run touches (a pair split across runs has a multiplicity
  that is not the largest of the runs' maxima);
- under `"sum"`, `min` and `max` of the stored pair sums: updated from the values the
  append writes; when a value it supersedes (§ 4.3) was the recorded `min` or `max`, both
  are recomputed at that commit from the per-pair column (one column, not the events),
  and a merge always recomputes them.

A column is **clean** — usable as a weight or a cost by every operation — when
`nan_count = 0`, `min ≥ 0` (or there is no value) and `sum` is finite. Since every value is
then non-negative, every partial sum is at most `sum`, so no pair weight, strength or total
can overflow. An operation on the whole graph decides from these statistics alone, before
any work: a clean column is accepted without reading it; any other is refused with
`InvalidInput` (exit status 3), naming the column and, from the statistics, the kind of
bad value (negative, NaN, infinite, or a total too large to sum). On a view the check is
made during the pass on the edges the view uses (`docs/api-sketch.md` § 2.13). NaN,
infinite and negative values are stored as they are: they are refused only by the
operations that need weights, and only when used.

### 2.8 The ingest report

Stored in the file's provenance (#28, #39) and printed in the result block
(`docs/ux-contract.md` G-11, G-12) and the file document (§ 4.7 of the contract):

| Count | Meaning | Where it appears |
|---|---|---|
| `rows` | rows read | `counts.rows` |
| `rejected` | rows that could not be read; 0 in 0.1 (§ 2.3) | `dropped.rejected`, left out when 0 |
| `self_loops` dropped | loop rows dropped (`"drop"`) | `dropped.self_loops`; `dropped: 4,201 self-loops` |
| `duplicates` dropped | repeated rows dropped (`"collapse"`) | `dropped.duplicates` |
| `merged` | repeated rows merged into an earlier row's edge (`"sum"`) | `counts.merged` |
| `edges` | events stored (`num_edges`) | `counts.edges` |
| `self_loops` kept | loop rows stored (`"keep"`); part of `edges` | `counts.self_loops` |
| `repeated` | rows stored as events that repeat a pair (`"events"`); part of `edges` | `counts.repeated` |
| `pairs` | pairs of the analytic graph, kept loops included (the `m` of § 3) | `counts.pairs` |
| `nodes` | nodes (§ 2.3) | `counts.nodes` |
| `isolates` | nodes with no stored edge (a node whose only edge is a kept loop is not an isolate) | `counts.isolates` |

**The report adds up exactly:**

    rows = rejected + dropped.self_loops + dropped.duplicates + merged + edges

and, inside `edges`, `edges = pairs + repeated` under `"events"` (with `repeated = 0`
under `"collapse"` and `"sum"`, where `edges = pairs`). The example of
`docs/ux-contract.md` § 4.4 satisfies it: 1,830,402,118 rows = 4,201 self-loops dropped
+ 1,830,397,917 edges. A test of the build (#35, #40) checks both identities on every
oracle graph.

## 3. The analytic graph

Let `w(p)` be the weight of pair `p` under the weight setting of the call (§ 4.1); with
`weight=False`, or for a kernel that is unweighted, `w(p) = 1`.

**Adjacency.** One matrix convention for everything, the one under which a loop adds 2 to
the undirected degree (Q1):

- directed graph: `A[u][v] = w((u, v))` for u ≠ v, and `A[v][v] = w((v, v))` for a kept
  loop;
- undirected graph, and the undirected projection of a directed graph:
  `A[u][v] = A[v][u] = w({u, v})` for u ≠ v, and `A[v][v] = 2·w({v, v})` for a kept
  loop. In the projection of a directed graph, `w({u, v}) = w((u, v)) + w((v, u))` when
  weighted (Q9) and 1 when not; `w({v, v}) = w((v, v))`.

**Degrees and strengths.** With `w = 1`:

    degree(v)     = Σ_u A[v][u] in the undirected projection = |N'(v)| + 2·[v has a kept loop]
    out_degree(v) = |N⁺(v) \ {v}| + [kept loop]          (directed graphs only)
    in_degree(v)  = |N⁻(v) \ {v}| + [kept loop]          (directed graphs only)

With the weight of § 4.1, the `weighted` column of `degrees()` is the strength in the
undirected projection, `s(v) = Σ_u A[v][u]`; on a directed graph that is out-strength plus
in-strength, so a kept loop of weight `w` adds `2w` there as in the undirected case.

**Size.** `m` = the number of pairs, kept loops included; `W` = Σ over pairs of `w(p)`,
each loop once. Then `Σ_v degree(v) = 2m` and `Σ_v s(v) = 2W` on the undirected
projection, and `Σ out_degree = Σ in_degree = m` on a directed graph. (The oracle's `m`
excludes loops and its invariant reads `Σ degree = 2m + 2·loops`; the two are the same
statement.)

## 4. Weights

### 4.1 Which weight

`weight=` as in `docs/api-sketch.md` § 2.4 and DECISIONS "Default weight":

| `weight=` | `w(p)` |
|---|---|
| `True`, a weight column declared at read | the pair's sum of that column (§ 4.3); under `"sum"` the stored sum; under `"collapse"` the kept row's value |
| `True`, no weight column | `mult(p)`: under `"events"` the number of events, under `"sum"` the `edges` column, under `"collapse"` 1 |
| a column name | the pair's sum of that numeric edge column |
| `False` | 1 |

The result records which: `meta["weight"]` is the column's name, `"edges"` or `None`.

### 4.2 Paths

`path(weight="col")` takes a **cost**, not the weight of § 4.1: the cost of a step over a
pair is the **smallest** non-null event cost of the pair (Q8), as NetworkX and igraph do;
under `"sum"` it is the stored sum (the events are gone) and under `"collapse"` the kept
row's value. A pair whose costs are all null cannot be used. Zero costs nothing. A kept
loop never shortens a path and is never a step.

### 4.3 Arithmetic

- A pair's sum is the **exact** sum of its event values, rounded once to `float64` (round
  to nearest, ties to even) for the kernels. It therefore does not depend on the order of
  the events, the direction they were read from, the threads or the regime, and an
  integer or decimal column loses precision only in that one rounding. (A sum of integers
  below 2⁵³ is exact in `float64` already; floating-point columns need an exact
  accumulator, which costs little because a pair's events are contiguous, § 2.5.) The
  oracle's references sum in row order in `float64`; they agree within the tolerances of
  `oracle/tolerances.json`, and `math.fsum` in the Python reference would make them
  agree exactly (§ 8).
- **Across runs** (a file built by appends, #111; not in 0.1). A pair's sum is the exact
  sum of its events **in every run**. The newest run that touches a pair stores that
  pair's aggregates over the events of all runs, recomputed from exact stored values (the
  older runs' events under `"events"`, the exact stored sum under `"sum"`), and they
  supersede the values older runs stored; a reader takes a pair's aggregates from the
  newest run that has the pair. A float `"sum"` graph has no exact stored values to
  recompute from (`round(round(a) + b)` need not equal `round(a + b)`), so
  `duplicates="sum"` with a floating-point weight makes a graph that cannot be appended
  (`Unsupported`, exit status 2); integer and decimal sums can. An append that would make
  a pair sum overflow its stored type (§ 4.3's widening is fixed when the file is built)
  is refused with `Unsupported` (2), naming the column, and the file is left unchanged;
  the way forward is a new file built from all the batches.
- Results that report a sum keep exact types (`docs/api-sketch.md` § 3.4): the
  `weighted` column of `degrees()`, the weight column of `neighbors()`, `edges_between()`
  and `p.edges()`, and the column stored by `"sum"` are `float64` for a floating-point
  input; `decimal128(38, 0)` for an integer input; `decimal128(38, s)` for
  `decimal128(p, s)` with p ≤ 26; `decimal256(76, s)` for a larger `decimal128` or any
  `decimal256(p, s)`. A sum that overflows its type is `InvalidInput` (3) naming the
  column; under `"sum"` the way forward is `duplicates="events"`.
- Kernels compute in `float64` (PageRank's state may be `float32` with
  `precision="low"`, `docs/api-sketch.md` § 2.4). The exact types are for what is
  reported, not for iteration.

### 4.4 Values a weight may take

| Value | Weighted PageRank, communities, a path cost | `degrees()` `weighted`, `neighbors()` and other reported sums | Unweighted operations |
|---|---|---|---|
| positive, finite | used | summed | ignored |
| zero | allowed: the pair counts nothing for PageRank and communities and costs nothing on a path; it is still a pair (a neighbour, a degree). A node whose out-weights are all 0 is dangling | summed | ignored |
| negative | `InvalidInput` (3) | summed as it is (a net amount is meaningful) | ignored |
| NaN, ±∞ | `InvalidInput` (3) | the IEEE result (NaN, ±∞) | ignored |
| null | skipped; all null → weight 0, or no step for a path | skipped; all null → 0 | ignored |

Reported sums never refuse: degrees are made with the graph and describe the data; a
refusal would hide the three counts with them (§ 9 lists the sentence of
`docs/api-sketch.md` § 2.4 that this narrows).

## 5. Self-loops in the kernels — the cases #36 decides

The oracle left five kernels unchecked on graphs with kept self-loops (`oracle/SEMANTICS.md`,
Q2 and "Kept self-loops"), because the libraries disagree or no rule had been taken. One
principle settles all of them:

> **A kept loop is a pair `{v, v}` of weight `w`, written into the adjacency with the
> convention of § 3 (2w on the diagonal of an undirected graph, w on a directed one).
> Measures of volume and flow — degrees, strengths, PageRank, modularity, the quotient
> graph — count it through that matrix. Measures of cohesion among *other* nodes — core
> numbers, triangles, clustering, transitivity — and of reachability — components,
> traversals, paths — ignore it, because a loop joins a node to no one.**

It agrees with Q1 (a loop adds 2 to the degree), keeps `Σ degree = 2m`, and is the
convention under which aggregating a graph into communities preserves modularity (§ 5.5).

The probe graph is `oracle/SEMANTICS.md`'s: triangle 0-1-2 with a loop at 0, undirected,
loops kept.

### 5.1 PageRank (S1)

    s(u)  = Σ_v A[u][v]                (out-strength; strength when undirected)
    D     = { u : s(u) = 0 }           (dangling)
    x(v)  = (1 − α)·t(v) + α·( Σ_u x(u)·A[u][v] / s(u)  +  t(v)·Σ_{u∈D} x(u) )

`α` is `damping`; `t` is the teleport vector, `1/n` everywhere or uniform over the
`personalize` set; dangling mass follows `t` (NetworkX and igraph agree, checked on a
personalized directed path with an isolate). The result is the fixed point (Q13).

**Rule.** A kept loop is the entry `A[v][v]`: `2w` on an undirected graph, `w` on a
directed one. A walker at `v` stays with probability `A[v][v] / s(v)`. A node whose only
edge is a loop is not dangling.

**Reason.** On a directed graph NetworkX and igraph agree (one arc `v → v`). On an
undirected graph they do not: NetworkX puts `w` on the diagonal, igraph `2w`. Taking
`2w` makes PageRank use the same matrix as the degree the user sees (Q1) and modularity
(§ 5.5): as `α → 1` on a connected undirected graph, PageRank tends to `s(v) / 2W`, the
strength shown by `degrees()`. With NetworkX's convention it would tend to a degree that
counts the loop once, which no other number in the product uses. It is also the
convention of the random-walk literature, where a loop contributes 2 to the degree, and
the one of the library that solves the system exactly (PRPACK).

Probe values: undirected `[19/40, 21/80, 21/80] = [0.475, 0.2625, 0.2625]` (igraph; NetworkX
gives `[0.416, 0.292, 0.292]`); with the loop's weight 3, `[38/63, 25/126, 25/126]`
(igraph, and NetworkX given the loop weight 6); directed, 3-cycle with a loop at 0,
`[0.480056, 0.254024, 0.265920]` (both).

### 5.2 Core numbers (S2)

**Rule.** Core numbers ignore kept loops. The k-core is the largest induced subgraph `H`
of the undirected projection with `|N'_H(v)| ≥ k` for every `v` in `H`; `core(v)` is the
largest `k` whose k-core contains `v`. Multiplicity and weight do not enter (A1, Q3).

**Reason.** NetworkX refuses graphs with loops (`NetworkXNotImplemented`); igraph counts
the loop's 2 in the degree, which makes a single node with a loop a 2-core, and a pair of
nodes with a loop each a 3-core (probe: `[2, 0]` and `[3, 3]`). A k-core is about how many
*other* members a node is tied to (Seidman 1983; Batagelj and Zaversnik 2003 define it on
simple graphs); a loop ties a node to nobody. It also keeps one number per node unchanged
between `self_loops="drop"` and `"keep"` for every cohesion measure, which is what a user
comparing the two expects. `core(v) ≤ degree(v)` still holds. #50's acceptance ("equal
NetworkX on oracle graphs") then holds with NetworkX run on the loop-free graph.

Probe: `[2, 2, 2]`.

### 5.3 Clustering (S3)

**Rule.** The loop's +2 does **not** enter `k(k − 1)`:

    k'(v) = |N'(v)|                    (distinct neighbours other than v, undirected projection)
    T(v)  = number of pairs {a, b} ⊆ N'(v), a ≠ b, with {a, b} a pair of the graph
    c(v)  = 2·T(v) / (k'(v)·(k'(v) − 1)),   and 0 when k'(v) < 2 (Q5)

So `k'(v) = degree(v) − 2` at a node with a kept loop.

**Reason.** NetworkX and igraph both ignore loops here (probe: `[1, 1, 1]`, and
`[1, 1, 1/3, 0]` with a pendant). Counting the loop would give the corner of a complete
triangle a clustering of `2/(4·3) = 1/6`: the coefficient would no longer be "the
fraction of my neighbours' pairs that are linked", and a node would be counted as its
own neighbour in a triangle it is not part of.

### 5.4 Transitivity (S4)

**Rule.**

    τ = Σ_v T(v) / Σ_v k'(v)·(k'(v) − 1)/2,   and 0 when there is no connected triple (Q6)

with `T` and `k'` as in § 5.3; `Σ_v T(v)` is three times the number of triangles.

**Reason.** Both libraries ignore loops (probe: 0.6 on the triangle with a pendant and a
loop, as without it), and transitivity must stay consistent with clustering and
triangles.

### 5.5 Modularity (S5) and the quotient graph (S6)

**Rule.** Modularity counts a kept loop as a pair inside its node's community, through the
matrix of § 3:

    Q = Σ_c [ L(c) / W  −  γ·( S(c) / 2W )² ],    and 0 when W = 0 (Q7)

where `L(c)` = Σ of `w(p)` over the pairs with both ends in `c`, **loops included** (each
loop once); `S(c)` = Σ_{v∈c} s(v), with the loop's `2w` in `s(v)`; `W` = Σ over all pairs
of `w(p)`, each loop once. This is `(1/2W)·Σ_ij [A_ij − γ·s_i·s_j / 2W]·δ(c_i, c_j)`
with `A[v][v] = 2w`. On a directed graph with `use_direction=False` (A2) it is computed on
the undirected projection (Q9 weights). With `use_direction=True` (#58), the
Leicht–Newman form:

    Q_d = (1/W)·Σ_ij [ A_ij − γ·s⁺_i·s⁻_j / W ]·δ(c_i, c_j),   A[v][v] = w

and CPM (#58) as `leidenalg`'s `CPMVertexPartition`, `Σ_c [ L(c) − γ·n_c·(n_c − 1)/2 ]`,
with the loops in `L(c)`. Weights as in § 4.1; `weight=False` gives every pair 1.

**The quotient graph** of a partition (the aggregation step of Louvain and Leiden, and
the oracle's `quotient` kernel) has one node per community, an edge between communities
`a ≠ b` of weight Σ of `w(p)` over the pairs between them, and **a loop at `a` of weight
`L(a)`** — kept loops of the input included. Σ of its weights is `W`.

**Reason.** NetworkX and igraph agree (probe: −0.125 for {0, 1} {2}, and −1/18 with the
loop's weight 3; directed −0.125 both). More importantly, this is the only choice under
which Louvain and Leiden are correct on such graphs: their aggregation turns the edges
inside a community into a loop of the aggregate node (Blondel et al. 2008), and the
modularity of the aggregated graph must equal the modularity of the partition it
represents. A loop that came from the input and a loop that came from aggregation must
therefore count the same way. With loops left out of `L` but not of `S`, or of both, the
identity breaks and so would the hierarchy of #62. The same argument makes the quotient
carry input loops on its diagonal; `c.table()`'s `internal_edges` counts a kept loop's
events (both ends are in the community), consistently.

### 5.6 Summary of the probe graph with loops kept

<!-- check: probe -->
| Kernel | Everygraph, loops kept | NetworkX | igraph | Rule |
|---|---|---|---|---|
| degree | [4, 2, 2] | [4, 2, 2] | [4, 2, 2] | Q1 |
| PageRank, undirected | [0.475, 0.2625, 0.2625] | [0.416, 0.292, 0.292] | [0.475, 0.2625, 0.2625] | S1, as igraph |
| PageRank, directed (3-cycle + loop at 0) | [0.480056, 0.254024, 0.265920] | same | same | S1 |
| core numbers | [2, 2, 2] | refuses | [2, 2, 2] (but [2, 0] for a lone loop) | S2, as NetworkX on the loop-free graph |
| triangles | [1, 1, 1] | [1, 1, 1] | [1, 1, 1] | already checked |
| clustering | [1, 1, 1] | [1, 1, 1] | [1, 1, 1] | S3 |
| transitivity | 1 | 1 | 1 | S4 |
| modularity, {0, 1} {2} | −0.125 | −0.125 | −0.125 | S5 |
<!-- /check: probe -->

The library values are from `oracle/semantics/probe.json` and from the same libraries
(NetworkX 3.7, python-igraph 1.0.0) on the cases the probe does not yet hold (the lone
loop's core numbers, the weighted loop, clustering and transitivity with a pendant); § 8
asks for them to be added to the probe.

## 6. What each operation does on each kind of graph

Every operation of `docs/api-sketch.md` § 6 that reads the graph's edges. Cells say what
the operation does; refusals give the exception and the exit status of
`docs/api-sketch.md` § 2.7 and § 5.4. "Pairs" means the analytic graph (§ 3); "events"
the stored edges. Columns:

- **Direction**: on a directed graph / on an undirected graph.
- **Repeated pairs**: under `duplicates="events"`, `"collapse"`, `"sum"` (§ 2.4).
- **Kept loop**: with `self_loops="keep"`. With `"drop"` there are no loops and nothing
  in this column applies.
- **Weights**: unweighted, weighted (§ 4.1), and bad weights (§ 4.4).

<!-- check: kernel-table -->
| Operation | Direction | Repeated pairs | Kept loop | Weights | Refusals |
|---|---|---|---|---|---|
| `g.degrees()` — `in_degree`, `out_degree`, `degree`, `weighted` | Directed: distinct out- and in-neighbours; `degree` = distinct neighbours in either direction (Q10). Undirected: `degree` only | Distinct neighbours, whatever the multiplicity. `weighted`: same under `events` and `sum`; `collapse` uses the kept rows | +1 to `in_degree` and `out_degree`, +2 to `degree` (Q1); `weighted` +2w | `weighted` present when the graph has a weight column; sums of § 4.3 with exact types; negative, NaN and ∞ are summed, never refused (§ 4.4) | none |
| `g.pagerank()` | Directed: out-edges. Undirected: both ways | `weight=True`: summed weight, or multiplicity (`edges` under `sum`, 1 under `collapse`); `weight=False`: 1 per pair | `A[v][v]` = 2w undirected, w directed (S1) | Zero weight: the pair counts nothing; a node with zero out-strength is dangling | Negative, NaN, ∞ weight: `InvalidInput` (3); personalize set empty: `ValueError` (2) |
| `g.components()` | Weak: the undirected projection. `strong=True`: strongly connected components | No effect | No effect | Unweighted | `strong=True` on an undirected graph: `Unsupported` (2) |
| `g.kcore()` | The undirected projection; `meta["direction"]` is `"ignored"` (Q3) | No effect | Ignored (S2) | Unweighted | none |
| `g.triangles()` — `triangles`, `clustering`, `transitivity`, `triangles_total` | The undirected projection (Q4) | No effect | Ignored: not a triangle, not in `k'` (S3, S4) | Unweighted | none |
| `g.bfs()` | `direction=`, default `"out"` (Q12). Undirected: every value means all neighbours | No effect | Never followed | Unweighted (hops) | Unknown source: `UnknownNode` (6) |
| `g.top()` | As the measure it ranks | As the measure | As the measure | As the measure | As the measure |
| `g.communities()` — modularity, CPM, `c.table()`, `c.modularity` | `use_direction=False` (default, A2): undirected projection, weights of both directions summed (Q9). `use_direction=True` (#58): Leicht–Newman | As PageRank. `internal_edges`, `external_edges` count stored edges (events; pairs under `collapse` and `sum`) | A pair inside its node's community: in `L(c)` once, `2w` in `S(c)`; in the quotient's loop (S5, S6); its events in `internal_edges` | Zero weight allowed; a graph whose total weight is 0 has modularity 0 (Q7) | Negative, NaN, ∞ weight: `InvalidInput` (3); `use_direction=True` on an undirected graph: `Unsupported` (2) |
| `g.map()` | As `communities()`; the layout ignores direction | As `communities()` | As `communities()`; a loop is not drawn as a link | As `communities()` | As `communities()` |
| `g.neighbors()` | `direction=`, default `"out"`. Undirected: all neighbours | One row per distinct neighbour: `edges` = multiplicity (`edges` column under `sum`, 1 under `collapse`), the summed weight, `first`, `last` | `v` is its own neighbour, once (`docs/api-sketch.md` § 2.13) | Reported sums (§ 4.4): never refused | Unknown node, one node asked: `UnknownNode` (6) |
| `g.khop()` | `direction=`, default `"out"` | No effect | Never followed; `v` is at distance 0 only | Unweighted | Unknown node: `UnknownNode` (6) |
| `g.path()` | Out-edges on a directed graph (Q12) | Hops: no effect. `weight=` cost: the smallest event cost of the pair (Q8); the stored sum under `sum`, the kept row under `collapse` (§ 4.2) | Never a step | A cost of 0 is free; null: the pair is not used | Negative, NaN, ∞ cost: `InvalidInput` (3); unknown end: `UnknownNode` (6) |
| `g.has_edge()` | Directed: an event `source → target`. Undirected: either way | True when the pair has at least one event | `has_edge(a, a)` is true exactly when `a` has a kept loop | — | Unknown node: `UnknownNode` (6) |
| `g.common_neighbors()` | Neighbours in either direction (the undirected projection) | One row per common neighbour; `edges_a`, `edges_b` are the events joining it to `a` and to `b`, both directions | `a` and `b` themselves are never listed | — | Unknown node: `UnknownNode` (6) |
| `g.edges_between()` | One row per pair with one end in each set; on a directed graph, per direction (`source`, `target` as stored) | `edges`, summed weight, `first`, `last` per pair | A loop at `v` is listed when `v` is in both sets | Reported sums (§ 4.4) | none |
| `g.filter()`, `g.window()`, `g.subgraph()` | Views of the same kind as the base graph | The view's events: multiplicities, sums and first/last are recomputed on the edges the view keeps; nodes as DECISIONS "Nodes in a view" | Kept loops in the view are kept loops of the view | Bad weights are found on the edges the view uses, during the pass (§ 2.7) | `window()` without a time column, or on a graph read with `duplicates="sum"`: `Unsupported` (2); start after end: `ValueError` (2); bad filter: `InvalidExpression` (2) |
| `g.num_nodes`, `g.num_edges` | — | `num_edges` counts stored edges: events, pairs under `collapse` and `sum` | A kept loop's events count | — | none |
| `g.nodes()`, `g.edges()` | `edges()`: one row per stored edge, `source`, `target` as read (an undirected edge once) | Under `sum`: the summed weight, `edges`, `first`, `last` | Loop rows appear as stored | Values as stored | none |
| `g.save()` | Writes the stored edges | As stored | As stored | As stored | none |
| `g.to_networkx()`, `g.to_igraph()` | `DiGraph` / `Graph`, directed igraph or not | One edge per pair with `edges` and the summed weight | A loop edge with its `edges` and weight. NetworkX's own PageRank on the result uses `w`, not `2w`, on an undirected loop (§ 5.1) | Values as summed | Above `max_edges`: `Unsupported` (2) |
| `g.to_scipy()` | Directed: `M[u][v]`; undirected: symmetric | One entry per pair: § 4.1's weight | Diagonal entry `w`, not `2w` — the convention of `networkx.to_scipy_sparse_array`, so the matrix round-trips through NetworkX | Values as summed, never refused | none |
<!-- /check: kernel-table -->

Every whole-graph operation on a graph with no nodes returns its usual columns and no
rows; on nodes without edges: PageRank `1/n`, one component and one community per node,
core 0, triangles 0, clustering 0, transitivity 0, modularity 0 (Q5–Q7; § 9 item 2).

## 7. Refusals

All refusals happen before any work on the whole graph (from the column statistics and
the graph's kind), and at the first bad edge met on a view. Exit statuses are those of
`docs/api-sketch.md` § 5.4.

<!-- check: refusals -->
| Case | Exception | Exit status |
|---|---|---|
| A row that cannot be read; a weight column that is not numeric; an exact sum that overflows its type | `InvalidInput` | 3 |
| A negative, NaN or infinite weight used by PageRank or communities, or as a path cost; a column whose total is not finite | `InvalidInput` | 3 |
| `components(strong=True)` or `communities(use_direction=True)` on an undirected graph | `Unsupported` | 2 |
| `window()` on a graph without a time column, or read with `duplicates="sum"` | `Unsupported` | 2 |
| `to_networkx()`, `to_igraph()` above `max_edges` | `Unsupported` | 2 |
| A filter that does not parse | `InvalidExpression` | 2 |
| A call about one node that is not in the graph or view | `UnknownNode` | 6 |
<!-- /check: refusals -->

The message of each follows `docs/ux-contract.md` § 6.1. For a bad weight: what (the
column), why (the statistic: "3 values are negative, the smallest −12.5"), and the ways
forward — `weight=False`, another column, or a view that leaves the bad edges out
(`g.filter(edges="value >= 0")`). For `window()` on a summed graph: why ("repeated edges
were summed when the graph was read, so the time of each event is gone") and the way
forward, reading the input again with `duplicates="events"`.

## 8. What the oracle checks

With the decisions of § 5, the oracle defines an answer for every kernel on every graph of
its corpus. The amendments it needs (to `oracle/SEMANTICS.md`, the Rust reference in
`crates/everygraph-oracle` and the Python references in `oracle/src`) are listed in the
pull request that adds this document, and are for the coordinator to apply; in short:

- PageRank, core numbers, clustering, transitivity and modularity stop being skipped on
  graphs with kept loops. The Rust reference adds the loop to `A` as in § 3 for PageRank,
  modularity and the quotient, and leaves core numbers, clustering and transitivity on
  the loop-free projection. The Python references give igraph the loop as an edge
  (igraph doubles an undirected loop itself) and NetworkX the loop with weight `2w` on an
  undirected graph for PageRank; core numbers stay on the loop-free graph for both
  libraries (igraph counts loops there).
- The `adjacency` kernel lists `v` in its own list once when it has a kept loop (it is
  its own neighbour, `docs/api-sketch.md` § 2.13), and the invariant `no_self_loops`
  becomes "no loop unless kept, never more than one per node".
- The `quotient` kernel puts kept loops on the diagonal (S6); `Σ quotient weights = m`
  then holds with the `m` of § 3.
- The oracle's graphs gain a `duplicates` field (`events`, `collapse`, `sum`), so that
  every option of § 2.1 is covered; the expected analytic graph under `sum` equals that
  under `events`, and under `collapse` has multiplicity 1 and the first row's values. The
  ingest report of § 2.8, and its two identities, become an output the oracle compares.
- New invariants: `Σ degree = 2m`, `Σ s = 2W` (with kept loops); modularity of a
  partition equals the modularity of its quotient graph under the singleton partition;
  PageRank sums to 1 with kept loops; core numbers, clustering and transitivity are the
  same with loops kept and dropped.

## 9. Disagreements found

Between the decision records and the specifications, and between this issue and its
neighbours. The ones in `docs/api-sketch.md` are corrected by the pull request that adds
this document; the others are for the coordinator.

1. **A kept loop in `degree`.** `docs/api-sketch.md` § 2.13 said a kept loop "counts 1 in
   `in_degree`, `out_degree` and `degree`". DECISIONS ("Graph semantics", Q1) says it adds
   2 to the undirected degree. The decision wins; § 2.13 is corrected.
2. **Modularity without edges.** `docs/api-sketch.md` § 2.13 (two rows) and § 3.5
   (`modularity: float  # NaN when there are no edges`) said NaN, printed `—`. DECISIONS
   (Q7) says 0, never NaN. The decision wins; both places are corrected.
3. **The file name.** The issue's acceptance and `docs/api-sketch.md` (§ 2.13, § 3.2,
   § 7) name `docs/spec/graph-model.md`; this document is `docs/spec/graph-semantics.md`,
   as the coordinator asked. The sketch's references are updated; the issue's acceptance
   line needs the new name.
4. **Weights that must be clean.** `docs/api-sketch.md` § 2.4 lists `degrees` among the
   takers of `weight=` and says weights "must be finite and not negative"; § 2.13 lists
   only PageRank, communities and path costs as refusing. Decided here (§ 4.4): reported
   sums (`degrees()`, `neighbors()`, `to_scipy()`) never refuse. § 2.4 is corrected.
5. **The oracle's quotient and adjacency with kept loops.** `oracle/SEMANTICS.md` checks
   the quotient graph "(a loop is not a pair)" and the adjacency without loops
   (`no_self_loops`). § 5.5 makes a kept loop a pair on the quotient's diagonal, and
   `docs/api-sketch.md` § 2.13 makes a node with a kept loop its own neighbour. Both
   oracle checks change (§ 8).
6. **Aggregates of `duplicates="sum"`.** The issue's scope lists "count, sum/min/max of a
   weight, first/last timestamp"; `docs/api-sketch.md` § 7 offers sum, count and
   first/last only. The sketch (with its amendment to this issue) wins: no min or max.
7. **Column types.** #37 (`m2-10-property-columns.md`) lists `decimal128` only; exact
   token amounts of 256-bit integers need `decimal256` (§ 2.6).
8. **Edge types.** The issue asks for "edge types as a small categorical column";
   `docs/api-sketch.md` § 3.1 reads only the source, target, weight and time columns and
   lists the rest as ignored. The storage is specified here (§ 2.6); the option to read
   such a column has no name and needs a preview.
9. **Repeated rows in the result block.** `docs/ux-contract.md` G-12 shows only
   `dropped: N self-loops`; with `"collapse"` the line also carries the dropped
   duplicates (key `duplicates`, open in the schema), and with `"sum"` the merged rows are
   in `counts.merged`. The printed words for both are not in the contract yet.

## 10. Decided here

Each with its reason; for `design/DECISIONS.md` (a row "Graph semantics, the cases left
to #36"), which this pull request may not edit.

1. **S1–S6, self-loops in the kernels** (§ 5): one adjacency convention (2w on the
   undirected diagonal); PageRank and modularity count loops through it, as igraph and
   (for modularity) both libraries do; core numbers, clustering and transitivity ignore
   them, as NetworkX does; the quotient carries input loops. *One matrix for every
   volume measure, so that PageRank, degree and modularity agree; cohesion is about
   other nodes; Leiden's aggregation is correct only this way.*
2. **The node set** includes identifiers whose only rows were dropped loops, as isolates
   (§ 2.3). *No identifier disappears uncounted, and an edge option does not change `n`.*
3. **Loops before duplicates** (§ 2.3). *A row is counted once, under the first treatment
   that applies.*
4. **`"sum"` is `"events"` for analysis** (§ 2.4): multiplicity from `edges`, weight from
   the stored sum. *The only loss should be the history the user chose to give up.*
5. **`window()` on a summed graph is refused** (§ 7). *A window over times that no longer
   exist would be a silent approximation.*
6. **A pair's sum is exact, rounded once** (§ 4.3), and exact input types stay exact in
   reported sums, with the widening rule of § 4.3. *Order-independent bits for the
   determinism promise; token amounts must add up to the cent.*
7. **Column statistics include `null_count` and `sum`**, beside min, max and NaN count
   (§ 2.7). *`sum` finite with `min ≥ 0` is what proves that no pair sum, strength or
   total can overflow, so a clean column is accepted without reading it.*
8. **Reported sums never refuse** (§ 4.4). *Degrees describe the data and are made with
   the graph; a refusal there would hide the three counts.*
9. **`to_scipy()` puts `w`, not `2w`, on the diagonal** (§ 6). *The matrix is for other
   tools, and NetworkX's convention lets it round-trip.*
10. **Dangling mass follows the teleport vector** (§ 5.1). *What NetworkX and igraph both
    do, checked on a personalized case.*
