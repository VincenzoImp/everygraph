# API sketch for v0.1: Python, Rust and the command line

Status: specification for v0.1 (issue #12). It is what #81 (Python API), #98 (command
line) and the kernel, query and map issues of M2–M7 build and test against. Its companion
is `docs/ux-contract.md` (issue #13), which owns everything about plans, progress,
interruption and the wording of messages; this document owns names, signatures, what
comes back and what it prints.

**Authority.** The shape of the API was approved in `design/preview/api.md` and
`design/preview/cli.md` with the recommended option of every choice
(`design/DECISIONS.md`); how things render is the typeset form in
`design/preview/style/terminal.html` and `notebook.html`. This document turns that into a
specification and does not reopen it. Where the previews were silent or disagreed, the
choice made here is listed in § 9 "Decided here", and nowhere else.

**Every figure in an example is invented.** Sizes, counts, scores and durations are
carried over from the approved previews, or made up in their style, to show shape. None
is a measurement, a benchmark or a target. Numeric defaults marked † are placeholders:
the implementing issue may change the value, not the name or the meaning.

**Three constraints the lab put on the shape** (`docs/FINDINGS.md` § F2, § F1, § G;
`docs/DESIGN.md` D2, D19, D20):

1. A call from Python into the engine has a fixed cost that makes per-node calls
   unusable at scale, so **nothing takes or returns one node at a time in a loop**:
   inputs are identifiers or columns of identifiers, outputs are columnar tables.
2. Results cross into Polars, pandas, pyarrow and DuckDB **without copying**, through the
   Arrow PyCapsule interface; no call returns a Python list or dict whose size grows with
   the number of nodes or edges.
3. Converting an existing `nx.Graph` costs more than the analysis it would speed up, so
   **loading through Everygraph is the easy path** and conversion goes the other way:
   from a small piece of an Everygraph graph to the small-graph tool.

The core must also stay runnable in a browser later (D2, D22): nothing in the object
model assumes a file system, a memory map or operating-system threads, and every long
operation is restartable.

**Contents.** 1 Ten user stories · 2 Conventions · 3 The object model (Python) ·
4 The Rust embedding API · 5 The command line · 6 Every public name, its issue and its
story · 7 Gaps · 8 Non-goals for 0.1 · 9 Decided here.

---

## 1. Ten user stories

Each story is the same task twice — Python, then the command line — with what is
printed. The stories are the acceptance test of #81 ("the ten user stories run as
written") and the outline of the tutorials (#88). Identifiers written like
`0x28c6…1d60` are shortened here; calls and commands take them in full.

Two example data sets are used throughout:

- `friends.csv`: a small text file with a header `user,friend`, one friendship per line.
- `transfers.parquet`: the file of the approved previews — a year of transfers between
  addresses, with columns `from`, `to`, `value`, `block_time`, `gas`, `nonce`, `input`.

### Story 1 — From a file to a table of communities

```python
import everygraph as eg

g = eg.read("friends.csv")               # columns found: user, friend
c = g.communities()                      # same answer every run, on any machine
c                                        # the summary
c.table().head(3)                        # one row per community
c.nodes().write("communities.parquet")   # one row per node: node, community
```

`eg.read` says what it understood (always, for a file) and what it made:

```text
friends.csv · 14.2 MB · ~1.0 M rows

  ids       integers · ~68 k distinct
  edges     undirected (the column names give no direction)
            duplicates kept as events · self-loops dropped
  columns   user → source · friend → target

✓ friends.csv · in memory · 68,304 nodes · 999,962 edges · 2 s
  dropped: 37 self-loops
  keep it: g.save("friends.eg")
```

```text
>>> c
Communities of friends.csv — Leiden, resolution 1.0, deterministic
  412 communities · modularity 0.7841 · 0 disconnected
  levels   68,304 nodes → 5,120 → 412
  largest  4,917 nodes (7.20%) · median 96 · 21 singletons
  weight   edges per pair (no weight column was declared)
  took     1 s
>>> c.table().head(3)
┌───────────┬───────┬────────────────┬────────────────┬───────┐
│ community │ nodes │ internal_edges │ external_edges │ share │
├───────────┼───────┼────────────────┼────────────────┼───────┤
│         0 │ 4,917 │         88,214 │          9,480 │ 7.20% │
│         1 │ 4,105 │         71,933 │          8,012 │ 6.01% │
│         2 │ 3,876 │         64,410 │          7,655 │ 5.67% │
└───────────┴───────┴────────────────┴────────────────┴───────┘
3 of 412 rows shown · sorted by nodes, largest first
```

```console
$ everygraph ingest friends.csv
friends.csv · 14.2 MB · ~1.0 M rows

  ids       integers · ~68 k distinct
  edges     undirected (the column names give no direction)
            duplicates kept as events · self-loops dropped
  columns   user → source · friend → target

✓ friends.eg · 5.1 MB · 68,304 nodes · 999,962 edges · 2 s
  dropped: 37 self-loops
  next: everygraph map friends.eg
$ everygraph run communities friends.eg --out communities.parquet
Communities of friends.eg — Leiden, resolution 1.0, deterministic
  412 communities · modularity 0.7841 · 0 disconnected
  levels   68,304 nodes → 5,120 → 412
  largest  4,917 nodes (7.20%) · median 96 · 21 singletons
  weight   edges per pair (no weight column was declared)
  took     1 s

✓ communities.parquet · 0.4 MB · 68,304 rows · 1 s
```

Issues: #32 (text reader), #34 (identifiers), #35 (build), #36 (what "undirected" and
"events" mean), #57 and #58 (communities), #62 (tables per community), #81, #83 (Arrow
out), #84 (in memory below a size), #98.

### Story 2 — A graph larger than memory

```python
g = eg.read("transfers.parquet", source="from", target="to",
            weight="value", time="block_time", directed=True)
pr = g.pagerank()
pr.top(5)
```

The plan is printed before the work starts, and nothing is asked
(`docs/ux-contract.md` § 3):

```text
transfers.parquet · 38.2 GB · 1,830,402,118 rows

  ids       hex addresses (20 bytes) · ~212.4 M distinct
  edges     directed · duplicates kept as events · self-loops dropped
  columns   value → weight · block_time → time · ignored: gas, nonce, input

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      build on disk — the graph is 4.6× the memory budget
  estimate  3 min 40 s · peak 5.9 GB · 61 GB scratch · result 9.3 GB

  reading      ████████████████  1,830,402,118 rows                 1 min 41 s
  identifiers  ████████████████  212,418,907 addresses                    38 s
  building     ████████████████  2,048 parts                              59 s
  checking     ████████████████  sorted · symmetric counts                12 s

✓ transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges · 3 min 30 s
  dropped: 4,201 self-loops
  reopen later: eg.open("transfers.eg")
```

`g.pagerank()` is long too, so it prints its own plan, then one line for its one phase:

```text
transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      edges on disk, read in passes — the graph is 1.2× the memory budget
  estimate  40 s · peak 3.4 GB · no scratch · result 1.7 GB in transfers.eg

  pagerank     ████████████████  38 passes over the edges                 41 s
```

```text
>>> pr.top(5)
┌────────────────────────────────────────────┬────────────┬──────┐
│ node                                       │ pagerank   │ rank │
├────────────────────────────────────────────┼────────────┼──────┤
│ 0x28c6c06298d514db089934071355e5743bf21d60 │ 0.00412907 │    1 │
│ 0xdac17f958d2ee523a2206206994597c13d831ec7 │ 0.00388114 │    2 │
│ 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 │ 0.00301775 │    3 │
│ 0x21a31ee1afc51d94c2efccaa2092ad1028285549 │ 0.00214466 │    4 │
│ 0x7a250d5630b4cf539739df2c5dacb4c659f2488d │ 0.00198203 │    5 │
└────────────────────────────────────────────┴────────────┴──────┘
top 5 of 212,418,907 rows by pagerank
```

```console
$ everygraph ingest transfers.parquet --source from --target to \
      --weight value --time block_time
$ everygraph run pagerank transfers.eg --top 5
node                                           pagerank  rank
0x28c6c06298d514db089934071355e5743bf21d60   0.00412907     1
0xdac17f958d2ee523a2206206994597c13d831ec7   0.00388114     2
0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48   0.00301775     3
0x21a31ee1afc51d94c2efccaa2092ad1028285549   0.00214466     4
0x7a250d5630b4cf539739df2c5dacb4c659f2488d   0.00198203     5

full result: transfers.eg (cached) · export with --out pagerank.parquet
```

The first command prints the same plan and phases as `eg.read` above, ending with
`next: everygraph map transfers.eg`. The second run of the same `pagerank` returns from
the file without computing.

Issues: #33 (Parquet reader), #34, #35, #40 (preflight), #45 (planner), #46 (reading the
edges in passes), #47 (PageRank), #39 (results kept in the file), #53 (never over the
budget).

### Story 3 — Neighbors and a shortest path that avoids the hubs

```python
g = eg.open("transfers.eg")
a = "0x28c6…1d60"

g.neighbors(a, direction="out", limit=5)
near = g.khop(a, k=2, limit=10_000)         # stops at 10,000 nodes and says so
near.truncated                               # True

hubs = g.top("degree", 100)
p = g.path("0x4f3a…91bc", "0x9e12…07aa", max_hops=5, avoid=hubs)
p
```

```text
>>> g.neighbors(a, direction="out", limit=5)
<stdin>:1: TruncatedWarning: neighbors() stopped at its limit of 5 rows;
0x28c6…1d60 has 1,204,877 out-neighbors. The result is partial and
marked truncated=True.
  to go further: g.neighbors("0x28c6…1d60", direction="out", limit=100)
┌─────────────┬───────┬────────────┬────────────┬────────────┐
│ neighbor    │ edges │ value      │ first      │ last       │
├─────────────┼───────┼────────────┼────────────┼────────────┤
│ 0x21a3…5549 │   412 │ 18,204.551 │ 2024-01-03 │ 2024-12-30 │
│ 0x3f5c…e2a1 │     7 │      2.100 │ 2024-02-11 │ 2024-02-19 │
│ 0x77e0…c1d4 │     1 │     12.400 │ 2024-03-02 │ 2024-03-02 │
│ 0x9a04…77b3 │    96 │  1,377.020 │ 2024-01-01 │ 2024-11-08 │
│ 0xdac1…1ec7 │ 2,118 │ 90,410.227 │ 2024-01-01 │ 2024-12-31 │
└─────────────┴───────┴────────────┴────────────┴────────────┘
5 rows · in graph order · cut short at limit=5 of 1,204,877 out-neighbors
>>> near = g.khop(a, k=2, limit=10_000)
<stdin>:1: TruncatedWarning: khop() stopped at its limit of 10,000 nodes,
part of the way through hop 2. The result is partial and marked
truncated=True.
  to go further: g.khop("0x28c6…1d60", k=2, limit=100_000)
>>> p
Path · 4 hops · found in 1.3 ms
  0x4f3a…91bc ── value 12.400 · 2024-03-02 ──▶ 0x77e0…c1d4
  0x77e0…c1d4 ── value  3.150 · 2024-03-02 · 3 events ──▶ 0xb81c…5520
  0xb81c…5520 ── value  3.149 · 2024-03-04 ──▶ 0x02fd…a9e3
  0x02fd…a9e3 ── value  3.100 · 2024-03-09 ──▶ 0x9e12…07aa
  avoided 100 hub nodes · 2 other shortest paths exist: p.alternatives()
```

The two warnings are Python's own display of a warning of category
`eg.TruncatedWarning` (§ 3.10): Python adds the `<stdin>:1: TruncatedWarning:` prefix,
and the rest is the message of `docs/ux-contract.md` § 6.2 (M-12). By Python's default
filter a warning is shown once per calling line. The total in the first footer
costs nothing: the out-degree of every node is stored when the graph is built (§ 3.2).
The second step of the path joins a pair with three transfers; the step shows the
earliest of them (§ 2.12).

```console
$ everygraph query neighbors transfers.eg 0x28c6…1d60 \
      --direction out --limit 5
$ everygraph query khop transfers.eg 0x28c6…1d60 --k 2 --limit 10000
$ everygraph query path transfers.eg 0x4f3a…91bc 0x9e12…07aa \
      --max-hops 5 --avoid-hubs 100
4 hops · 1.3 ms
0x4f3a…91bc → 0x77e0…c1d4 → 0xb81c…5520 → 0x02fd…a9e3 → 0x9e12…07aa
avoided 100 hub nodes · 2 other shortest paths exist
```

The first two commands print the same tables without borders on standard output, and
the same "cut short" message on standard error, in its command-line form.

Issues: #90 (point queries, `limit`, `timeout`, `truncated`), #91 (paths, hop limit,
avoid-set, alternatives, the choice among equal paths), #38 (in-edges for directed
paths), #92 (the same queries with the graph on disk), #50 (degrees stored at ingest, for
`top("degree")` and the footer's total).

### Story 4 — The graph between two dates

```python
march = g.window("2024-03-01", "2024-03-31")
big   = g.filter(edges="value > 10")
both  = march & big       # nothing is read yet: a view is its definition

both
both.num_edges          # one pass over the edges, then known
c = both.communities()  # everything that works on a graph works on a view
```

```text
>>> both
View of transfers.eg · not counted yet
  window   2024-03-01 … 2024-03-31
  edges    value > 10
>>> both.num_edges
4118306
>>> both
View of transfers.eg · 1,204,551 nodes · 4,118,306 edges
  window   2024-03-01 … 2024-03-31
  edges    value > 10
>>> c
Communities of transfers.eg (view) — Leiden, resolution 1.0, deterministic
  4,118 communities · modularity 0.7120 · 0 disconnected
  levels   1,204,551 nodes → 96,410 → 4,118
  largest  88,204 nodes (7.32%) · median 41 · 902 singletons
  weight   value, summed per pair
  note     direction was ignored (use_direction=False)
  took     9 s
```

Making a view reads nothing, and its text form never starts a pass: it shows the
definition, and the counts only once they are known. The first access to `num_nodes` or
`num_edges` is a planned operation like any other — one pass over the edges, the phase
`counting`, with its plan and progress line when it is long. This one is not (its
estimate is under `T_PLAN`, 5 s, `docs/ux-contract.md` § 1 and P-3), so it prints
nothing but the answer. The counts are then kept with the view (§ 3.3).

```console
$ everygraph run communities transfers.eg \
      --window 2024-03-01 2024-03-31 --edges "value > 10"
```

It prints the plan block, the `communities` phase line and the same summary.

Issues: #93 (views: filter, compose, mask or materialize, counts), #94 (time windows),
#37 (the columns a filter reads), #45 (the planner picks how the view is held), #57.

### Story 5 — A map, and sharing it

```python
m = g.map()            # builds whatever is missing, after printing the plan
m                      # in a notebook: the interactive map
m.highlight(g.pagerank().top(100))
m.selection.nodes()    # what was clicked, back in Python
m.export("shared.egmap", reveal="summary")
```

At a prompt, where no map can be drawn:

```text
>>> m
Map of transfers.eg · 6 zoom levels · 83 regions at the top level
  the 74 largest communities and 9 aggregates holding the other 31,886
  selection  none
  see it     in a notebook, or: everygraph open transfers.eg
  share it   m.export("shared.egmap", reveal="summary")
>>> m.selection.nodes()          # after selecting community 4 on the map
┌─────────────┬───────────┬───────────┬────────────┬───────────┐
│ node        │ community │ in_degree │ out_degree │ degree    │
├─────────────┼───────────┼───────────┼────────────┼───────────┤
│ 0x40ec…bbdf │         4 │ 2,118,406 │  2,004,551 │ 3,870,114 │
│ 0x8eb8…ab28 │         4 │ 1,371,930 │  1,368,104 │ 2,512,877 │
│ 0x99c9…4be1 │         4 │   904,077 │    688,412 │ 1,433,190 │
│ 0xa3a7…0eec │         4 │   570,345 │    569,902 │ 1,019,466 │
│ 0x3ee1…a585 │         4 │   312,880 │    201,226 │   498,011 │
│ …           │         … │         … │          … │         … │
│ 0xfe21…09c4 │         4 │         1 │          0 │         1 │
└─────────────┴───────────┴───────────┴────────────┴───────────┘
6 of 7,305,118 rows shown · community 4 · sorted by degree, largest first
```

```console
$ everygraph map transfers.eg
transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      edges on disk, read in passes — the graph is 1.2× the memory budget
  estimate  about 4 min · peak 5.6 GB · 12 GB scratch · result 4.1 GB

  communities  ████████████████  31,960 · modularity 0.603          2 min 11 s
  hierarchy    ████████████████  6 zoom levels                            18 s
  layout       ████████████████                                           41 s
  tiles        ████████████████  1,204,118 tiles                     1 min 2 s

✓ transfers.egmap · 4.1 GB · 4 min 12 s
  open it:   everygraph open transfers.eg
  share it:  everygraph map transfers.eg --out shared.egmap --reveal summary
$ everygraph open transfers.eg
Serving on http://127.0.0.1:7461 (this computer only) — opening your browser.
Ctrl-C to stop.
$ everygraph map transfers.eg --out shared.egmap --reveal summary
```

The last command reuses everything already computed (its phase lines say `kept`) and
writes a second map file, with `viewer.html` beside it, that holds regions and counts
but no identifiers; `--reveal full` is the other value, for a map that shows every
identifier. The working map `transfers.egmap` beside the graph is the full one,
for this computer; a map written for sharing always says what it reveals, because
`--reveal` (`reveal=` in Python) is required and has no default (§ 3.8).

Issues: #57 and #60 (communities, also with the edges on disk), #63 (the hierarchy the
map needs), #67 (layout), #68 and #69 (tiles), #75 (export and what it reveals), #74
(`open`), #76 (the notebook widget, selection and highlight), #78 (the caption).

### Story 6 — A result joined with my own table

```python
import duckdb
import polars as pl

labels = pl.read_csv("labels.csv").with_columns(  # address, label
    pl.col("address").str.to_lowercase())         # as Everygraph gives it
pr = g.pagerank()

top = pl.from_arrow(pr.top(1000)).join(       # no copy of the result
    labels, left_on="node", right_on="address", how="left")

members = g.communities().nodes()             # node, community
duckdb.sql("""
    select m.community, count(*) as labelled, min(l.label) as example
    from members m join labels l on m.node = l.address
    group by m.community order by labelled desc limit 10
""")
```

Nothing here is printed by Everygraph: the two results are Arrow tables that Polars and
DuckDB read directly (§ 3.4). Addresses come back as lowercase text with their `0x`
(§ 2.2), while a file of labels often holds them with mixed-case checksums (EIP-55); the
address column is therefore lowercased before the join, which would otherwise miss those
rows without any error.

```console
$ everygraph run pagerank transfers.eg --top 1000 --out top.parquet
✓ top.parquet · 52 kB · 1,000 rows · 1 s
$ duckdb -c "select l.label, p.pagerank
             from 'top.parquet' p join 'labels.csv' l
               on p.node = lower(l.address)
             order by p.pagerank desc limit 10"
```

The scores are already stored in `transfers.eg` (story 2), so writing the thousand
highest is short and prints only its result line (`docs/ux-contract.md` P-3). Writing
all 212,418,907 rows (`--out pagerank.parquet` without `--top`) is long: it prints its
plan block, the `pagerank` phase as `kept` and a `writing` phase before the `✓` line.

Issues: #83 (zero-copy export, tested by buffer address), #95 (DuckDB reads the tables),
#47, #57.

### Story 7 — Handing a piece of the graph to NetworkX

The existing NetworkX code stays as it is; Everygraph finds the piece worth studying and
hands it over.

```python
import networkx as nx

ball = g.khop("0x28c6…1d60", k=2, limit=5_000)     # node, distance
ego  = g.subgraph(ball)                            # a view of 5,000 nodes
G    = ego.to_networkx()                           # nx.DiGraph, original ids
nx.betweenness_centrality(G)                       # unchanged NetworkX code

c = g.communities()
g.subgraph(c[4]).to_networkx()                     # 98 million edges: refused
```

```text
Unsupported: to_networkx() is limited to 1,000,000 edges;
this view has 98,340,221.

  Ways forward:
    g.subgraph(c[4]).to_networkx(max_edges=100_000_000)    lift the guard
    g.subgraph(c[4]).save("community-4.parquet")           hand over a file

Nothing was changed.
```

The guard needs the view's edge count; here it is already known from `c.table()`
(`internal_edges` of community 4), so nothing is read before refusing. The file offered
is the edge table in Parquet, which pandas, Polars, DuckDB and `nx.from_pandas_edgelist`
read at this size; GraphML of a hundred million edges would not be practical.

```console
$ everygraph export transfers.eg --window 2024-03-01 2024-03-02 \
      --edges "value > 1000" --out big-transfers.graphml
✓ big-transfers.graphml · 3.4 MB · 9,204 nodes · 12,877 edges · 3 s
$ python -c "import networkx as nx
G = nx.read_graphml('big-transfers.graphml')"
```

On the command line the hand-over is a file, and the piece is chosen by a filter or a
time window; picking a community or a neighborhood as the piece is Python-only in 0.1
(§ 7).

Issues: #87 (subgraph and the bridge to small-graph tools, with size guards), #85
(converters and file formats), #93 (the view), #90 (`khop`).

### Story 8 — An operation that does not fit the budget

```python
g.explain("communities", memory="4GB")      # look before running: a Plan
g.communities(memory="4GB")                 # or just try: it is refused first
```

```text
>>> g.communities(memory="4GB")
Traceback (most recent call last):
  File "<stdin>", line 1, in <module>
InsufficientBudget: communities() needs 5.1 GB for per-node state
(212,418,907 nodes × 24 bytes); the budget is 4.0 GB.
The edges stay on disk either way.

  Ways forward:
    g.communities(memory="6GB")       the smallest budget that works

Nothing was changed.
```

With enough memory, `g.explain("communities")` shows how it would run:

```text
>>> g.explain("communities")
Plan for communities() on transfers.eg — Leiden, resolution 1.0, deterministic
  fits     peak 5.6 GB of 8.0 GB budget

  1  keep per-node state              memory            5.1 GB
  2  move nodes between communities   edges from disk   0.5 GB   about 12 passes
  3  merge communities and repeat     memory          < 0.4 GB   4 levels
  4  write the result                 disk                   —

  why      212,418,907 nodes × 24 bytes = 5.1 GB of state
           the edges (9.3 GB) stay on disk and are read each pass
  estimate about 2 min
```

```console
$ everygraph run communities transfers.eg --memory 4GB
✗ Not enough memory for this step.

  communities needs 5.1 GB for per-node state (212,418,907 nodes × 24 bytes).
  The budget is 4.0 GB. The edges stay on disk either way.

  Ways forward:
    --memory 6GB            the smallest budget that works

Nothing was changed.
$ echo $?
4
```

A refusal lists only ways forward that exist. Community detection has no lower-memory
variant in 0.1 (`design/DECISIONS.md`, "A low-memory variant of community detection"),
so a larger budget is the one offered; the refusal of `pagerank()` also offers
`precision="low"`, which #47 builds. The planner never switches to such a variant by
itself (§ 2.11).

`--dry-run` prints the plan instead of running, and `--json` gives either as a document
(`docs/ux-contract.md` § 3.6 and § 6.1).

Issues: #31 (the memory accountant that refuses instead of failing), #45 (planner,
`explain`, the smallest budget), #53 (the no-out-of-memory test matrix), #60, #81, #98.

### Story 9 — An interrupted job, resumed

```python
g = eg.read("transfers.parquet", source="from", target="to",
            weight="value", time="block_time", directed=True)
# Ctrl-C, or "interrupt the kernel", during the second phase
```

```text
  reading      ████████████████  1,830,402,118 rows                 1 min 41 s
  identifiers  █████████░░░░░░░  61% · 5.6 M/s · about 15 s left          23 s
^C
Stopped at your request, during identifiers.

  kept      reading · 31 GB of scratch in .everygraph-7c1e
  dropped   identifiers at 61% — that phase starts again from its beginning
  written   nothing: there is no transfers.eg yet

  continue:  run the same call again
  discard:   delete the directory .everygraph-7c1e
Traceback (most recent call last):
  File "<stdin>", line 1, in <module>
KeyboardInterrupt
>>>
```

The stop block goes through the progress channel, on standard error, before anything
is raised; the last three lines are Python's own report of the `KeyboardInterrupt`,
which Everygraph raises unwrapped once its workers have stopped and its scratch is
tidied (`docs/ux-contract.md` § 5.2). In a script the traceback names the script's
line instead of `<stdin>`, and the script ends there unless it catches the exception.
The scratch directory is hidden beside the file being made (§ 2.4, `temp_dir`), here
the current directory.

The same call again continues from the last finished phase:

```text
transfers.parquet · 38.2 GB · 1,830,402,118 rows
  Continuing the run stopped 6 min ago: the file has not changed since.

  reading      ████████████████  1,830,402,118 rows                       kept
  identifiers  ████████████████  212,418,907 addresses                    38 s
  building     ████████████████  2,048 parts                              59 s
  checking     ████████████████  sorted · symmetric counts                12 s

✓ transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges · 1 min 49 s
  dropped: 4,201 self-loops
  reopen later: eg.open("transfers.eg")
```

```console
$ everygraph ingest transfers.parquet --source from --target to \
      --weight value --time block_time
  reading      ████████████████  1,830,402,118 rows                 1 min 41 s
  identifiers  █████████░░░░░░░  61% · 5.6 M/s · about 15 s left          23 s
^C
Stopped at your request, during identifiers.

  kept      reading · 31 GB of scratch in .everygraph-7c1e
  dropped   identifiers at 61% — that phase starts again from its beginning
  written   nothing: there is no transfers.eg yet

  continue:  run the same command again
  discard:   delete the directory .everygraph-7c1e
$ echo $?
130
$ everygraph ingest transfers.parquet --source from --target to \
      --weight value --time block_time
transfers.parquet · 38.2 GB · 1,830,402,118 rows
  Continuing the run stopped 6 min ago: the file has not changed since.

  reading      ████████████████  1,830,402,118 rows                       kept
  identifiers  ████████████████  212,418,907 addresses                    38 s
  building     ████████████████  2,048 parts                              59 s
  checking     ████████████████  sorted · symmetric counts                12 s

✓ transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges · 1 min 49 s
  dropped: 4,201 self-loops
  next: everygraph map transfers.eg
```

(The first transcript and the first command are shown from the phase lines onward; the
summary and plan of story 2 come before them.) The behaviour is identical in a notebook,
where the `continue:` line reads `run the cell again` (`docs/ux-contract.md` § 5.4).

Issues: #40 (checkpoints at phase boundaries, the manifest, stale scratch), #28 (a graph
file that is never half-written), #81 (`KeyboardInterrupt`), #98.

### Story 10 — A small graph built from arrays

```python
import numpy as np

source = np.array([0, 0, 1, 2, 2, 3])
target = np.array([1, 2, 2, 3, 4, 4])

g = eg.from_edges(source, target)       # undirected, in memory, nothing written
g
g.triangles()
g.save("tiny.eg")                       # only if it should outlive the session
```

```text
>>> g
Graph in memory · 5 nodes · 6 edges
  ids       integers
  edges     undirected
>>> g.triangles()
┌──────┬───────────┬────────────┐
│ node │ triangles │ clustering │
├──────┼───────────┼────────────┤
│    0 │         1 │     1.0000 │
│    1 │         1 │     1.0000 │
│    2 │         2 │     0.3333 │
│    3 │         1 │     1.0000 │
│    4 │         1 │     1.0000 │
└──────┴───────────┴────────────┘
5 rows · in graph order
```

A graph built from arrays stays in memory while it is small (below 100 MB of edges †);
a larger one is spilled to a hidden `.eg` in the scratch directory and removed when the
graph object is released, unless `g.save()` keeps it (#84).

A graph in memory lives inside a Python (or Rust) process, so the command line has no
`from_edges`. Its equivalent starts from a file, however small:

```console
$ printf 'a,b\n0,1\n0,2\n1,2\n2,3\n2,4\n3,4\n' > tiny.csv
$ everygraph ingest tiny.csv
tiny.csv · 28 B · 6 rows

  ids       integers · 5 distinct
  edges     undirected (the column names give no direction)
            duplicates kept as events · self-loops dropped
  columns   a → source · b → target

✓ tiny.eg · 8.2 kB · 5 nodes · 6 edges · 4 ms
  next: everygraph map tiny.eg
$ everygraph run triangles tiny.eg
node  triangles  clustering
   0          1      1.0000
   1          1      1.0000
   2          2      0.3333
   3          1      1.0000
   4          1      1.0000
```

Issues: #84 (graphs from arrays and tables, kept in memory below a size), #51
(triangles), #83 (arrays in without copying).

---
## 2. Conventions

Stated once, applied everywhere. A method's table in § 3 lists only what is particular
to it.

### 2.1 Names

| Rule | Examples |
|---|---|
| The package, the import and the command are `everygraph`; the alias `eg` exists in Python code only. | `import everygraph as eg`, `everygraph ingest …` |
| Algorithms and queries are methods on the graph, with short names. | `g.pagerank()`, `g.components()`, `g.kcore()` |
| A method is named for what you get, and how it is computed is a parameter. | `g.communities(method="leiden")` |
| "nodes" and "edges" in every name; the user's own nouns only in printed summaries. | `num_nodes`, `c.nodes()`; `212,418,907 addresses` |
| "communities" are what the algorithm finds; "regions" are the areas of a map. The word "groups" is used for neither. | `g.communities()`, `m.hierarchy` |
| A part of a graph is a "view". | `g.filter()`, `g.window()`, `g.subgraph()` |
| "budget" means memory and nothing else; a cap on a query is `limit`, a count of nodes or rows a query returns, never memory or time, and `timeout` (seconds). | `memory="8GB"`, `limit=10_000`, `timeout=10` |
| The degrees of a node are `in_degree`, `out_degree` and `degree`, in tables, in `top()` and in filters. | `g.top("degree", 100)`, `nodes="in_degree > 100"` |
| One word for one thing: a repeatable result is "deterministic", in every name and every printed line. | `deterministic=True`; `Leiden, resolution 1.0, deterministic` |
| Files: `.eg` is a graph, `.egmap` is a map. | `transfers.eg`, `transfers.egmap` |
| A call's subjects — what it is about: the data, a file, one or two nodes or node sets, a measure, a number of rows, a level, the bounds of a window, an operation — come first and are positional, at most two of them. Every other parameter is keyword-only. | `g.khop(a, k=2, limit=10_000)`, `g.top("degree", 100)`, `pr.top(5)`, `g.window(start, end)` |
| A command-line option is the parameter's name with dashes. | `max_hops=5` ↔ `--max-hops 5` |
| Column names of result tables are lowercase words with underscores, are part of the API, and are what the command prints as headers. | `node`, `pagerank`, `internal_edges` |

### 2.2 Identifiers in and out

- **In.** A node is named by its identifier as it appears in the input: a `str`, an
  `int`, or `bytes`. Text that was recognized as hexadecimal is matched without regard
  to case. Shortened forms (`0x28c6…1d60`) are for display and are never accepted.
- **Out.** Every result carries the original identifiers, in a column called `node`
  (or `neighbor`, `parent`, `source`, `target` where a row is about another node or an
  edge). Its Arrow type is the type of the input column: an integer column gives
  integers of the same type, a text column gives text (`utf8`), a binary column gives
  binary, and arrays given to `eg.from_edges` give their own type.
- **Hexadecimal identifiers.** Text recognized as hexadecimal is stored as fixed-width
  bytes (#34) and comes back as text: **lowercase**, with `0x` when the input used a
  prefix (`0x` or `0X`) and with none otherwise. UUIDs come back lowercase, with hyphens.
  Look-ups accept any case (`design/DECISIONS.md`, "Identifier columns: hex prefixes,
  UUIDs, stray keys"). A table of the user's that holds the same addresses in mixed case
  (EIP-55 checksums) must be lowercased before a join (story 6).
- **Node sets.** Wherever a set of nodes is expected (`avoid=`, `subgraph()`,
  `highlight()`, `neighbors()`, `personalize=`), any of these is accepted — called
  `Nodes` in the signatures:
  - one identifier, or a Python sequence of identifiers;
  - a NumPy array or an Arrow array of identifiers;
  - any table with a column named `node`: an Everygraph table, a Polars or pandas
    DataFrame, a pyarrow table, a DuckDB relation;
  - a `Community` (`c[0]`) or a `Selection` (`m.selection`).
- **Unknown identifiers.** A call about one node raises `UnknownNode`. A call about
  many returns a row of nulls for each unknown identifier and counts them in
  `meta["unknown"]`; it does not fail.
- **Graph order.** Every graph has an order of its nodes: the order of the rows of
  `g.nodes()`. It is fixed when the file is built, never changes afterwards, and is the
  same every time the same input is read with the same parameters, whatever the threads
  or the budget. A view's order is its base graph's order restricted to the view's
  nodes, and `save()` keeps it. Graph order is what ties are broken by (§ 2.12). The
  engine's internal node numbers are not part of the API and appear nowhere.

### 2.3 Units and numbers

| Quantity | In arguments | In results and JSON | Printed |
|---|---|---|---|
| Memory, sizes | `"8GB"`, `"500MB"`, `"1.5GB"`; units `kB MB GB TB` (powers of 10) and `KiB MiB GiB TiB` (powers of 2), case-insensitive; an `int` is bytes | bytes (`*_bytes`) | `8.0 GB` (GB = 10⁹ bytes) |
| Time limits and durations | seconds, `int` or `float` | seconds (`*_seconds`) | `38 s`, `1 min 41 s`, `1.3 ms` |
| Times | below | Arrow timestamp, UTC; or the integer type of the time column | `2024-03-02` |
| Counts | `int` | unsigned integers (below) | exact, with separators: `1,830,397,917` |
| Scores | — | `float64` (`float32` with `pagerank(precision="low")`) | as many decimals as distinguish the rows shown |

**Times** (`design/DECISIONS.md`, "Time windows"). A time — a bound of `g.window()`, or
a value compared with a time column in a filter — is one of:

- **a date without a time** — `"2024-03-31"`, a `datetime.date`, a `numpy.datetime64`
  with the unit `D`: the whole day, from its first instant to its last;
- **anything with a time** — `"2024-03-31T12:00"`, `"2024-03-31 00:00"`, a
  `datetime.datetime`, a `pandas.Timestamp`, a `numpy.datetime64` of a finer unit: that
  instant, even when it is midnight;
- **an `int`**: a value in the time column's own unit, compared without conversion — a
  block number for an integer column, a count of the timestamp's unit since the epoch
  for an Arrow timestamp column.

Times without an offset or a zone are UTC; an offset or a zone is honoured. Text or a
date on an integer time column is a `TypeError`. A window is closed at both ends, so a
date as its start counts from that day's first instant and a date as its end up to that
day's last: `g.window("2024-03-01", "2024-03-31")` is the whole of March, while
`g.window("2024-03-01", datetime(2024, 3, 31))` stops at midnight on the 31st. In a
filter a date stands for its whole day in the same way: `t >= '2024-03-01'` from its
first instant, `t <= '2024-03-31'` to its last, `t < d` before the day, `t > d` after
it, `t == d` within it.

**Integers in results** are unsigned: 32-bit in 0.1 where they count or name nodes
(`component`, `community`, `region`, `parent` of a region, `core`, `distance`, `level`,
`rank`, the degree columns), 64-bit where they count edges or triangles. The width of
the 32-bit ones is not frozen: it follows the node-id width (`docs/DESIGN.md` D3) and
grows with the 64-bit build (#114).

Printed numbers follow `docs/ux-contract.md` § 2.6 on every surface. Numbers inside
tables handed to other tools are plain values with no formatting.

### 2.4 Parameters that mean the same everywhere

| Parameter | Command line | Type | Default | Taken by | Meaning |
|---|---|---|---|---|---|
| `memory` | `--memory SIZE` | size (§ 2.3) | half of the memory free when the operation is planned | everything that is planned | The memory budget: the most memory the engine may own. Pages of the graph file that the operating system maps from disk are not counted, because it can drop them at any moment. An operation whose per-node state cannot fit is refused before it starts (`InsufficientBudget`); an operation that starts stays inside the budget. A budget larger than the machine's memory in all is a `ValueError` (a usage error, exit status 2, on the command line); one larger than what is free now is honoured, and the plan says so (`docs/ux-contract.md` P-16, P-17). The budget changes how an operation runs, never its result (§ 2.11). |
| `threads` | `--threads N` | `int` ≥ 1 | the physical cores the process may use | everything that is planned | The most worker threads to use. The planner may use fewer (a small input, a kernel that does not scale) and, only when `threads` is not given, may add the second hardware thread of each core for kernels that gain from it (`docs/DESIGN.md` D13). It never changes a deterministic result (§ 2.11). |
| `temp_dir` | `--temp-dir DIR` | path | see the next column | everything that is planned | Where scratch goes (`design/DECISIONS.md`, "Where scratch goes"): the **scratch root**, the directory in which the job's hidden directory `.everygraph-ID` is created (`docs/ux-contract.md` § 1, § 5.3). `EVERYGRAPH_TEMP_DIR` gives it when the argument does not. When none is given, the scratch root is the directory of the output file; for an operation that writes no file, the directory of the graph file; for a graph held in memory, the system temporary directory, where scratch on a memory-backed file system counts against the memory budget. That is rule P-18 of `docs/ux-contract.md`, and it holds for any scratch root on `tmpfs`, which `/tmp` is on many systems. |
| `deterministic` | `--deterministic` / `--fast` | `bool` | `True` | `communities`, `map` | `True`: the result is repeatable as § 2.11 promises. `False` is the faster mode: the result is as good on average but not repeatable. It is labelled `fast` wherever the result is printed or stored, and is never returned in place of a deterministic one. Every other operation is always deterministic and does not take the parameter. |
| `seed` | `--seed N` | `int` ≥ 0 | `0` | `communities`, `map` | Selects one of the equally valid answers of a randomized method. Same seed, same answer (when deterministic). |
| `precision` | `--precision full\|low` | `"full"` or `"low"` | `"full"` | `pagerank` | `"low"` keeps the per-node state in 32 bits instead of 64, halving it, at a small cost in accuracy (#47). The result is labelled `precision low` and stored under its own key. It is never chosen by the planner: when `"full"` does not fit, the refusal offers `"low"` (§ 2.11). No other operation has a lower-precision variant in 0.1, and none takes the parameter. |
| `weight` | `--weight NAME` / `--no-weight` | `bool` or column name | `True` | `pagerank`, `communities`, `degrees` (the `weighted` column), `to_scipy` | How much each pair of connected nodes counts. `True`: the sum over the pair's edges of the weight column declared when the graph was read (`eg.read(weight=…)`), if one was declared; otherwise the number of edges between the pair. A name: the sum of that numeric edge column. `False`: every connected pair counts 1. The result always says which was used: `meta["weight"]` is the column's name, `"edges"` or `None`, and the summary of communities prints it (`design/DECISIONS.md`, "Default weight"). Weights used by `pagerank` and `communities` must be finite and not negative (§ 2.13); the `weighted` column of `degrees` and `to_scipy` report sums as they are and never refuse (`docs/spec/graph-semantics.md` § 4.4). (`path` differs: § 3.7.) |
| `limit` | `--limit N` | `int` ≥ 1 | per call (§ 3.2) † | `neighbors` (rows per node), `khop` (nodes returned), `path` (nodes visited), `common_neighbors`, `edges_between` (rows) | A count of nodes or rows a query returns (for `path`, the nodes its search visits), never memory or time. Reaching it ends the call with a partial result marked `truncated` (§ 2.5), not with an error. These five are the point queries that can be cut short (`docs/ux-contract.md` § 4.1). |
| `timeout` | `--timeout SECONDS` | seconds | per call (§ 3.2) † | `khop`, `path` | The most time a traversal may take. Same outcome as `limit`; a result cut short by time is not repeatable (§ 2.11). |
| `dry_run` | `--dry-run` | `bool` | `False` | everything that is planned | Plan only: return the `Plan` (§ 3.9), run nothing, write nothing. |
| `progress` | — | `bool` or a function of one `dict` | `True` | everything that is planned | `True`: print the plan and progress (`docs/ux-contract.md` § 2.1). `False`: print nothing. A function: receive the events of `docs/ux-contract.md` § 4.6 instead, on the calling thread (§ 2.9). |
| `overwrite` | `--overwrite` | `bool` | `False` | everything that writes a file | Replace a file that Everygraph would otherwise refuse to replace (§ 2.7, "Files that exist"). |

"Everything that is planned" is: `eg.read`, `eg.from_edges`, every whole-graph
algorithm, the first count of a view's nodes or edges and `degrees()` on a view
(§ 3.3), `g.map()`, `g.save()`, `t.write()` and `m.export()`. The five parameters
`memory`, `threads`, `temp_dir`, `dry_run` and `progress` are written `**run` in the
signatures of § 3.

**Where a default comes from**, first match wins: the argument of the call; the value
given to `eg.read` or `eg.open` when the graph was obtained; the environment
(`EVERYGRAPH_MEMORY`, `EVERYGRAPH_THREADS`, `EVERYGRAPH_TEMP_DIR`); the built-in
default. The plan always says which (`docs/ux-contract.md` § 3.4, the `budget` line).

### 2.5 Results that are partial

Decided (`design/preview/choices.md` E1): always marked in the result itself, never only
in the documentation.

| | A result cut short |
|---|---|
| Happens when | a traversal reached its `limit` or its `timeout` |
| Attribute | `.truncated` is `True` |
| Detail | `meta["truncated"]`: `by` (`"limit"` or `"timeout"`), `limit` (its value), `rows` returned, `where` it stopped (`"hop 2"`), `timeout_seconds` for a cut by time, and `total` only when it is known without further work (a stored degree, story 3) |
| Printed with the table | in the footer: `cut short at limit=10,000, in hop 2` |
| Printed when it happens | in Python, a warning of category `eg.TruncatedWarning` through Python's `warnings` module, so a script can silence it or turn it into an error (`warnings.simplefilter("error", eg.TruncatedWarning)`); in a notebook, the same warning, and the result's card carries the approved "Cut short." note of `notebook.html` cell 8; on the command line, the "cut short" warning on standard error (`docs/ux-contract.md` § 6.2, M-12) |
| In `--json` and in Arrow | `"truncated": {…}` in the document; the same object in the schema metadata of the Arrow stream, so it survives into Parquet |
| Exit status | 0 |
| Repeatable | when cut by `limit`, yes (§ 2.12 says which rows are kept); when cut by `timeout`, no (§ 2.11) |

A table of which only some rows are *displayed* is not cut short: its footer says
`6 of 31,960 rows shown` and the object holds every row.

No operation in 0.1 returns an approximate result. When a sampled method arrives (#52,
after 0.1), its results will be marked in the result itself in the same places, under
names chosen then.

### 2.6 What things print

- Every public object has a text form (`repr`, `str`, `print`) and a notebook form (a
  card; the map is a widget). The two show the same facts in the same order; the text
  form is what this document specifies exactly, the card is typeset in
  `design/preview/style/notebook.html`.
- Text forms obey the terminal rules of `docs/ux-contract.md` § 2: at most 80 columns,
  the same number formats, ASCII fallbacks.
- **Tables** print as a grid: the header row holds the column names; text is
  left-aligned, numbers right-aligned with thousands separators; one footer line always
  follows. A table of more than 10 rows shows its first five rows, a row of `…`, and its
  last row. The footer says how many rows are shown of how many, how they are ordered
  (§ 2.12), and anything from § 2.5. If the grid would pass 80 columns, identifiers are
  shortened in the middle — on display only.
- **Nothing printed starts work.** A text form shows what is known; a total that would
  need a pass over the graph (a view's counts, the distinct neighbors of a node in a
  view) is left out until it has been computed.
- On the command line the same table has no borders: columns aligned with spaces on a
  terminal, tab-separated with a header row when piped, one document with `--json`
  (§ 5.3).
- Nothing prints by itself except through `progress` (§ 2.4) and the warnings of § 2.5:
  an object prints when the prompt or the notebook displays it.

### 2.7 Errors

All errors raised by the engine derive from `eg.EverygraphError`. Each carries the whole
message as its text, in the pattern of `docs/ux-contract.md` § 6.1 — what, why with the
arithmetic, the ways forward as code that can be copied, and what state things were left
in — and the same content as attributes.

| Exception | Also a | Raised when | Particular attributes | Exit status |
|---|---|---|---|---|
| `InsufficientBudget` | — | The per-node state of an operation does not fit the memory budget. Always before any work. | `step`, `needs_bytes`, `budget_bytes`, `smallest_budget_bytes` | 4 |
| `InsufficientDisk` | — | The scratch or output directory lacks room for the plan. Before any work; or during a run if the disk fills. | `step`, `needs_bytes`, `available_bytes` (free on that disk), `path` | 4 |
| `InvalidInput` | — | The input cannot be read as described: a row with the wrong number of fields, a value that is not of the column's type, a missing column, an unknown format, a file with no bytes. Or the data cannot be used as asked: a weight that is negative, NaN or infinite where an operation needs weights (§ 2.13). | `problems` (each with `file`, `line`, `problem`, `column`, `excerpt`), `invalid_rows`, `rows_checked`; for a weight, `column` | 3 |
| `InvalidExpression` | `ValueError` | The text of a filter does not parse or names a column the graph lacks. Raised by `g.filter()` itself, not later. | `expression`, `position` | 2 |
| `UnknownNode` | `LookupError` | A call about one node names an identifier that is not in the graph or the view. | `node` | 6 |
| `Unsupported` | — | The operation is not defined for this graph: `window` without a time column or on a graph read with `duplicates="sum"`, `strong=True` or `use_direction=True` on an undirected graph; or it is above a size guard (`to_networkx`). | `operation`, `reason` | 2 |
| `OutputExists` | `FileExistsError` | The output path holds a file that Everygraph may not replace (below). Before any work. | `path`, `holds` (`"other graph"` or `"not ours"`) | 1 |
| `CorruptFile` | — | A graph or map file fails its integrity checks, or was written by a format this version cannot read. | `path`, `section` | 5 |

`UnknownNode` derives from `LookupError` and not `KeyError`, because `KeyError` quotes
its argument and would print the multi-line message as one quoted string. Its exit
status, 6, lets a script that loops over identifiers tell "not in the graph" from a
mistake in the command (`design/DECISIONS.md`, "Exit status for 'no such node'").

Common attributes of every `EverygraphError`, with the names and meanings of the error
document of `docs/ux-contract.md` § 6.1: `message` (the first sentence); `why` (the
explanation, with its numbers); `arithmetic` (`count`, `unit`, `bytes_each`, or
`None`); `ways_forward`, a list of `eg.Way` objects with the fields `command` (what the
command line shows), `python` (the same as a call), `note` (its consequence, possibly
empty) and `needs_bytes` (or `None`) — the keys of a way forward in that document;
`changed` (`False` when nothing was written); `kept` (the finished phases kept after a
failure, C-8, with their `path` and `bytes`, or `None`); `machine_memory_bytes` (or
`None`); and `to_dict()`, which is that document. Sizes in attributes are named as in
that document: `needs_bytes`, `budget_bytes` (the memory budget),
`smallest_budget_bytes` — which is `None` when no budget on this machine would work
(`docs/ux-contract.md` P-15) — and `available_bytes` only for what is free on a disk or
in the machine.

**Files that exist.** Everygraph replaces a file only when it can tell what the file
holds:

- `eg.read(path)` and `everygraph ingest FILE` make `NAME.eg`, where `NAME` is the
  input's name with every recognized extension removed (`transfers.csv.gz` and
  `transfers.parquet` both give `transfers.eg`). If that file exists and was built from
  the same input, unchanged, with the same parameters, it is the cache: it is opened,
  and a notice says so (`docs/ux-contract.md` § 6.3); a stopped run of the same call is
  continued (story 9). If it holds anything else — a graph built from another input, or
  from this one with other parameters or before it changed — it is never replaced: the
  call raises `OutputExists`, whose ways forward are `out=` / `--out` with another name,
  and `overwrite=True` / `--overwrite`.
- Any other write (`--out`, `t.write`, `g.save`, `m.export`, `everygraph export`)
  replaces a file that Everygraph wrote and can recognize — a `.eg` of the same graph, a
  `.egmap`, a Parquet or Arrow file carrying the `everygraph` metadata key (§ 3.4) — and
  refuses with `OutputExists` for anything else (a CSV, a GraphML file, a Parquet file
  from elsewhere, a `.eg` of another graph) unless `overwrite=True` / `--overwrite`.
  Nothing is promised that cannot be detected: a CSV written by Everygraph cannot be
  told from anyone else's, so writing over it needs `overwrite=True`.

A file that is replaced is written beside it and renamed over it at the end, so it is
never half-written (`docs/ux-contract.md` § 5.2, C-1).

Outside the taxonomy on purpose:

- A wrong argument — an unknown keyword, `k=-1`, `direction="sideways"`, a missing
  `reveal=`, a memory budget larger than the machine — is Python's own `TypeError` or
  `ValueError`. On the command line the same mistakes are usage errors, reported as
  `UsageError` (a class of the command line only, so that `--json` still gives an error
  document, `docs/ux-contract.md` § 6.1) with exit status 2.
- Errors of the operating system stay Python's own `OSError` subclasses and pass
  through as they are: `FileNotFoundError`, `PermissionError`, `IsADirectoryError`. On
  the command line they are reported as `IoError` (a class of the command line only,
  with the `path` concerned, so that `--json` gives an error document), exit status 1.
- `KeyboardInterrupt` is never wrapped (`docs/ux-contract.md` § 5.2). The library
  installs no signal handlers: Python's own handler turns `Ctrl-C` into
  `KeyboardInterrupt`, and the engine notices it at its next check (§ 2.9). The command
  installs its own, for SIGINT and SIGTERM.
- Reaching a `limit` or a `timeout` is not an error (§ 2.5).

The exact text of a refusal, as an example of the pattern, is in story 8.

### 2.8 What is kept, and where

| What | Where | For how long |
|---|---|---|
| The graph | `NAME.eg` beside the input (`NAME` as in § 2.7), when `eg.read` is given a file of at least 100 MB † or `out=` is given; otherwise in memory, spilled to a hidden `.eg` in scratch above 100 MB † (#84) | until deleted |
| Results on the whole graph of a file: PageRank, components, core numbers, triangles, degrees (made with the graph), communities with their levels, the map's hierarchy and layout | inside the graph file, as sections appended and committed with a record of the parameters that produced them (#39; `docs/adr/0027-single-file-builds.md`, `docs/spec/eg-container.md` § 8) | until the result is replaced (`compact` then reclaims its bytes) or the file is deleted |
| Results on a view, and `bfs()` from a node | for the session only: held by the process, in memory or in its scratch when large, and found again by any call that asks for them | until the process ends; kept only when written out (`t.write`, `--out`) |
| Results on a graph in memory | in memory | while the graph object lives |
| The counts and degrees of a view | with the view object | while it lives |
| The working map | `NAME.egmap` beside the graph; for a view or a graph in memory, in scratch until exported | until deleted |
| Scratch of a running or stopped job | a directory `.everygraph-ID` in `temp_dir` | removed on success (`docs/ux-contract.md` § 5) |
| Point queries, plans, tables derived in Python (`.top()`, `.head()`) | nowhere | — |

- Results on views are not written into the graph file, so that views made on the fly —
  a window, `g.subgraph(g.khop(…))` — never make it grow (#39: "large temporary
  results should not bloat the graph file"). `bfs()` is kept for the session too: it is
  one table per source node. `g.top()` derives from the stored measure.
- A result is looked up before it is computed. The key is the graph's content (its
  identity, `docs/spec/eg-container.md` § 6), the view's definition, the algorithm and
  its version, every parameter that can change the result, `seed`, `precision`, and
  whether it is deterministic. `memory`, `threads`, `temp_dir` and `progress` are not
  part of the key, because they cannot change the result (§ 2.11).
- A hit is not silent: the phase is shown as `kept`, `meta["cached"]` is `True`, and the
  command prints `full result: transfers.eg (cached)`.
- There is no hidden cache directory in a home folder (`api.md` Choice 1).
- If the graph file cannot be written (a read-only medium), results are kept for the
  session and a warning says so once (`read-only`, `docs/ux-contract.md` § 6.2).

**Approximate results** (decided 2026-11-25, `design/DECISIONS.md`, "Approximate
results"). No operation in 0.1 is approximate; when the sampled methods of #52 arrive,
these are the names § 2.5 left open:

- `.approximate` is `True`, and `meta["approximate"]` holds `method`, `seed`, `samples`,
  the kind of guarantee, `epsilon` and `delta` or the relative standard deviation, and
  `runs`. The same keys are recorded in the result's provenance, so a stored approximate
  result says what it is. The footer says it too, as it does for a result cut short.
- The precision parameters — HyperLogLog registers, ε, δ, the sample schedule — are
  parameters of the call, and so part of the key above. They are never derived from
  `memory` or `threads`, which stay outside the key: otherwise two budgets would return
  different results under one key.
- Where no bound is proven (HyperBall's harmonic and closeness centrality), the result
  says "no proven bound" and gives only an empirical spread over `runs` when more than
  one run was asked for.

### 2.9 Threads and processes

- `Graph`, `View`, every table, `Communities`, `Path` and `Plan` are immutable once
  made (a view's counts are filled in once, under a lock). They can be shared between
  threads and used from several at once.
- A call that computes releases the interpreter lock while the engine works, so other
  Python threads run, and point queries from many threads run in parallel. It takes the
  lock again only for short moments: to call a `progress` function; at each check for
  an interrupt, where it asks Python whether a signal has arrived (at the engine's chunk
  boundaries, by the one rule of `docs/ux-contract.md` C-4: at least every 64 MiB of
  input or 10⁶ items of work); and to build the object it returns. Python delivers signals to the main thread only, so `Ctrl-C` stops a call
  made on the main thread; a call on another thread runs to its end.
- A function passed as `progress` is called on the thread that made the call, with the
  lock held, never concurrently. An exception it raises stops the operation cleanly
  (`docs/ux-contract.md` § 5.2).
- A `Map` shown in a notebook has state (its selection, its highlights) and belongs to
  the thread that displays it.
- Several processes may read one graph file at once. Adding a result to it is done by
  one writer at a time, under a lock on the file. A second writer is refused with
  `ResourceBusy`, which names the file; it does not wait
  (`plan/issues/m2-22-single-writer-lock.md`; `design/DECISIONS.md`, "A second writer
  on a graph file is refused"). A reader never sees a half-written result
  (`docs/ux-contract.md` § 5.2, C-1). The container itself does not lock
  (`docs/spec/eg-container.md` § 11): the lock across processes is issue
  `m2-22-single-writer-lock`, and a reader in another process that maps the file stays
  safe during a commit only once maps cover the committed range alone (issue
  `m2-21-range-maps`).
- Two threads of one process that add results to the same file are serialized the same
  way, by a lock the process holds for the file; both succeed, one after the other. If
  both computed the same result, it is stored once and both get it.

### 2.10 What is stable in 0.1

Promised not to change without a deprecation in a later 0.x release:

- the Python names, parameters and defaults of § 3 that are not marked † or ‡;
- the column names of result tables and their types (§ 3.4), except the width of the
  32-bit integer columns (§ 2.3);
- the exception and warning classes of § 2.7 and § 3.10 and their attributes;
- the commands, options and exit statuses of § 5;
- the JSON documents, under their `schema` number: the plan, event, error, stop and
  file documents of `docs/ux-contract.md` (§§ 3.6, 4.6, 6.1, 5.4, 4.7) and the result
  document of § 5.3;
- the promise of § 2.11, within one version of Everygraph.

These names freeze when 0.1 is released, after the pre-launch review by outside
practitioners (#105) has been through them; until then a change is made here, with its
reason in § 9.

Not promised:

- the exact wording and layout of printed text (it is covered by golden tests, and it
  may still improve);
- values marked †, and names marked ‡ (taken from an issue file, not shown in an approved
  preview);
- identical results across versions of Everygraph (an algorithm's version is part of the
  cache key, so an upgrade recomputes rather than mixing);
- the Rust API of § 4, which follows Rust's rules for 0.x crates;
- the `.eg` and `.egmap` formats before their specifications say so (#28, #68).

### 2.11 What "deterministic" promises

`design/DECISIONS.md`, "What 'repeatable' promises", in the words of this document:

> A deterministic run — the default — returns the same result for any number of
> threads, on any platform the release supports, for any memory budget, and whichever
> way the planner executes it: in memory or out of core, a view held as a mask or
> materialized, a result computed now or read back from the file.

- "The same result" means the same rows, in the same order, with the same values to the
  bit, compared by the result hash of #54. "The same input" means the same bytes read
  with the same parameters (graph order included, § 2.2).
- The planner never substitutes a lower-precision variant by itself. When the
  full-precision state does not fit, it refuses and offers the variant as a way forward
  (`pagerank(precision="low")`); a variant that was asked for is labelled and stored
  under its own key.
- Not covered: a result cut short by `timeout`, since where it stops depends on the
  machine's speed (a cut by `limit` is covered); results with `deterministic=False`;
  results of different versions of Everygraph (§ 2.10).
- "Any platform the release supports" is the list of `docs/DESIGN.md` D14.
- For a file built by appends (segments, after 0.1), results depend on its batches, not
  on how its runs are merged or compacted (`design/DECISIONS.md`, "What 'repeatable'
  promises, for files built by appends"; ADR 0028).
- If a kernel cannot meet this, the promise is narrowed by an ADR, not silently. The
  identity tests that hold the kernels to it are listed in § 7, "Amendments".

### 2.12 Order of rows, and ties

Every result has a defined order, so that a deterministic result is the same table and
not only the same set of rows. Ties are broken by graph order (§ 2.2).

| Result | Order |
|---|---|
| One row per node: `pagerank()`, `components()`, `kcore()`, `triangles()`, `degrees()`, `bfs()`, `c.nodes()`, `g.nodes()` | graph order |
| Components, communities (`component`, `community` numbers) | numbered from 0 by decreasing size, at every level; of two of equal size, the one containing the earlier node in graph order comes first |
| Levels of `Communities` | numbered from 1 at the coarsest, so level 1 is the partition. The printed `levels` line runs the other way, from the nodes (finest) to the partition (coarsest): `68,304 nodes → 5,120 → 412` is the nodes, then level 2, then level 1 |
| `c.table()`, `.sizes()` | by number of nodes, largest first; equal sizes by the first column (for components and communities this is their numbering) |
| `g.top()`, `t.top()` | largest value first, ties in graph order; `rank` is 1, 2, 3 … with no shared ranks (tied rows take consecutive ranks in graph order: "ordinal"); rows whose value is null are left out |
| `khop()` | by distance, then by discovery order. A node's parent is the first node of the previous distance, in that distance's order, with an edge to it (in the direction searched); within one distance, nodes come in the order of their parents' positions in the previous distance's order, then of their positions in that parent's list of neighbors, which is in graph order. The traversal stops as soon as `limit` rows exist and returns them: the same nodes on every run and at any thread count, at a cost bounded by what is returned |
| `bfs()`, the `parent` column | the earliest node in graph order among those one hop closer to the source that have an edge to the node (in the direction searched) |
| `path()` | the same path on every run. Which of several shortest paths is returned, and the order of `p.alternatives()`, are defined and tested by #91. A step shows the earliest edge of that pair inside the view — its weight and its time — and says how many edges join the pair when there are more than one (`· 3 events`); with `weight=` (a cost) it shows the cheapest, the earliest among equal costs |
| `neighbors()` | neighbors in graph order. With several nodes, one block per node in the order the nodes were given (an identifier given twice appears once, at its first place), and `limit` applies to each node |
| `common_neighbors()`, `edges_between()` | graph order of `neighbor`; of `source`, then `target` |
| `c.of()`, `Hierarchy.of()` | level 1 first (the top), down to the finest |
| `Hierarchy.table()` | by level, then `region` |
| `m.selection.nodes()` | by `degree`, largest first, ties in graph order |

### 2.13 Edge cases

| Case | What happens |
|---|---|
| An input file with no bytes | `InvalidInput`: there is no header and no row to read. |
| A header and no rows; a Parquet file or a table with no rows | A graph with 0 nodes and 0 edges, summarized and written like any other. |
| A graph or view with no nodes | Every whole-graph result is a table with its usual columns and no rows. `communities()` finds 0 communities, with modularity 0 (`design/DECISIONS.md`, "Graph semantics": never NaN). |
| Nodes but no edges (a node filter that keeps none of the edges) | `pagerank()`: every node `1/n` — every node is dangling and its rank is spread evenly, so scores still sum to 1. `components()` and `communities()`: one per node, numbered in graph order; modularity 0. Core number 0; triangles 0. |
| `clustering` of a node with fewer than two neighbors | 0.0, as in NetworkX. |
| A weight that is negative, NaN or infinite, in the column used by `pagerank`, `communities` or as a path cost | `InvalidInput` (exit status 3), naming the column: it is a property of the data. The column statistics that the build records (`docs/spec/graph-semantics.md` § 2.7) — among them each numeric edge column's minimum, maximum, sum and count of NaN — give the fast answer: a column with no bad value anywhere is accepted without reading anything. On the whole graph a column with bad values is refused before any work. When the operation runs on a view of such a graph, the check is made on the edges the view actually uses, during the pass, and the operation fails at the first bad one it meets, naming it; a view that leaves out every bad edge runs. Zero is allowed: a pair of weight 0 counts nothing for PageRank and communities, and costs nothing on a path. |
| A null weight | Skipped in sums (a pair whose weights are all null weighs 0); a path does not use an edge whose cost is null. |
| `direction=` on an undirected graph | Every value means the same: all neighbors. Not an error. |
| `kcore()` or `triangles()` on a directed graph | Direction is ignored: each connected pair counts once; `meta["direction"]` is `"ignored"`. |
| `communities(use_direction=True)` or `components(strong=True)` on an undirected graph | `Unsupported`: there is no direction to use. |
| `g.window()` on a graph without a time column, or on a graph read with `duplicates="sum"` (the time of each event is gone) | `Unsupported` (`docs/spec/graph-semantics.md` § 7). |
| A window whose start is after its end | `ValueError`. |
| `path(a, a)` | Found, 0 hops, one node. |
| `khop(a, k=0)` | The node itself, at distance 0. |
| An empty node set | `subgraph()`: an empty view. `avoid=`: no effect. `personalize=`: `ValueError`. `highlight()`: clears, like `None`. |
| `top(n)` with `n` larger than the number of rows; `n=0` | All the rows; no rows. |
| A self-loop kept with `self_loops="keep"` | The node is its own neighbor once (`neighbors()`, `has_edge(a, a)`). It adds 1 to `in_degree` and `out_degree` and 2 to `degree` (`design/DECISIONS.md`, "Graph semantics"). What each algorithm does with it is the table of `docs/spec/graph-semantics.md` § 6 (#36). |
| `a & b` with views of different graphs | `ValueError`. |
| A node filter on a view (`v.filter(nodes="degree > 10")`) | Reads the degrees inside `v`, computed by a planned pass (§ 3.3). |


---
## 3. The object model (Python)

Ten kinds of object. The signatures are written as Python stubs; they are the
specification of names, parameter order, types and defaults. In them:

```python
Node      = str | int | bytes                # one identifier (§ 2.2)
Nodes     = ...                              # any node set of § 2.2
Size      = str | int                        # "8GB", or bytes (§ 2.3)
Time      = str | int | datetime.date | datetime.datetime | numpy.datetime64
Direction = Literal["out", "in", "both"]
Measure   = Literal["degree", "in_degree", "out_degree", "weighted",
                    "pagerank", "core", "triangles", "clustering"]
TableLike = ...   # anything with the Arrow PyCapsule stream interface:
                  # pyarrow, Polars, pandas, a DuckDB relation, an eg.Table

class Run(TypedDict, total=False):           # written **run below (§ 2.4)
    memory: Size
    threads: int
    temp_dir: str | os.PathLike
    dry_run: bool
    progress: bool | Callable[[dict], None]
```

Any call that takes `**run` returns a `Plan` instead of its usual result when
`dry_run=True`. ‡ marks a name taken from an issue file rather than an approved preview;
† marks a placeholder value.

| Object | What it is | Obtained from |
|---|---|---|
| `Graph` | An immutable graph: in a `.eg` file, or in memory. | `eg.read`, `eg.open`, `eg.from_edges` |
| `View` | Part of a graph, defined by a filter, a time window or a node set. Behaves as a graph. | `g.filter`, `g.window`, `g.subgraph`, `view & view` |
| `Table` | A result: a columnar table that is an Arrow table, with a few graph-aware helpers. | every algorithm and query |
| `Communities` | The result of community detection: a summary, a table per level, the membership of every node. | `g.communities()` |
| `Community` | One community, as a set of nodes. | `c[i]` |
| `Path` | A shortest path between two nodes, or the fact that none was found. | `g.path()` |
| `Map` | The zoomable map of a graph: a `.egmap` file and, in a notebook, a widget. | `g.map()` |
| `Hierarchy` | The tree of regions a map is drawn from. | `m.hierarchy` |
| `Selection` | What is selected on a displayed map. | `m.selection` |
| `Plan` | What an operation would do and cost; possibly a refusal. | `g.explain()`, `dry_run=True` |

### 3.1 Module functions

```python
def read(
    data: str | os.PathLike | TableLike,
    *,
    source: str | None = None,
    target: str | None = None,
    weight: str | None = None,
    time: str | None = None,
    directed: bool | None = None,
    self_loops: Literal["drop", "keep"] = "drop",
    duplicates: Literal["events", "collapse", "sum"] = "events",
    ids: Literal["auto", "string"] = "auto",
    out: str | os.PathLike | None = None,
    overwrite: bool = False,
    **run: Unpack[Run],
) -> Graph: ...

def open(
    path: str | os.PathLike,
    *,
    memory: Size | None = None,
    threads: int | None = None,
    temp_dir: str | os.PathLike | None = None,
) -> Graph: ...

def from_edges(                                                    # ‡ #84
    source: ArrayLike,
    target: ArrayLike,
    *,
    weight: ArrayLike | None = None,
    time: ArrayLike | None = None,
    directed: bool = False,
    self_loops: Literal["drop", "keep"] = "drop",
    duplicates: Literal["events", "collapse", "sum"] = "events",
    **run: Unpack[Run],
) -> Graph: ...

def from_scipy(matrix, *, directed: bool | None = None) -> Graph: ...  # ‡ #84

__version__: str
```

| Function | Details | Implemented by |
|---|---|---|
| `eg.read` | Reads a file or a table of edges and returns a graph. **`data`**: a path — text or CSV, optionally `.gz` or `.zst` (#32); Parquet (#33); GraphML, GEXF, Matrix Market (#85); a `.eg` file, which is simply opened — or any table (`TableLike`), read without copying (#83, #95). **`source`, `target`**: column names; when omitted, the columns named `from`/`to` or `source`/`target`, else the first two. **`weight`, `time`**: optional column names; they become the graph's weight and time columns. Other columns are ignored, and listed. **`directed`**: when `None`, directed if the columns are named like a direction (`from`/`to`, `source`/`target`), undirected otherwise; the summary says which and why. **`self_loops`**: `"drop"` (default, `choices.md` A3) drops edges from a node to itself and counts them on the `dropped:` line; `"keep"` keeps them. **`ids`**: how identifiers are read. `"auto"` (default) detects their kind — integers, hexadecimal, UUIDs, base58, fixed-size binary, else text — from a sample of the first keys; a key that does not fit the detected kind is an invalid row, reported with its line. `"string"` keeps every key as text, as it is written (`design/DECISIONS.md`, "Identifier columns: hex prefixes, UUIDs, stray keys"; `everygraph_ingest::ids`, #34). **`duplicates`** — what to do with several edges between the same pair (on an undirected graph `a,b` and `b,a` are the same pair): `"events"` (default, A1) keeps every edge as a separate event, and the algorithms analyse the pair once with its weight; `"collapse"` keeps one edge per pair, the first in input order with its columns, and counts the others as dropped — smaller, history gone; `"sum"` keeps one edge per pair carrying the sum of the weight column, the number of edges it replaces (`edges`) and the earliest and latest time (`first`, `last`). The summary's `edges` line says which (`duplicates kept as events`, `duplicates collapsed`, `duplicates summed`: `docs/ux-contract.md` § 3.4) and whether self-loops were dropped or kept. **`out`**: where to write the `.eg`. Default: beside the input, named as in § 2.7, when the input is a file of at least 100 MB †; smaller inputs and tables stay in memory. An existing `.eg` is opened, continued or refused as § 2.7 says; `overwrite=True` replaces it. `memory`, `threads` and `temp_dir` given here become the defaults of the returned graph. Prints the summary always, and the plan and progress when long (`docs/ux-contract.md` § 3, § 4). | #81; #32, #33, #34, #35, #36, #40, #83, #84, #85 |
| `eg.open` | Opens a `.eg` file. Reads its header and its table of contents only, whatever the file's size (#28). Raises `CorruptFile` if the quick integrity check fails (#41). | #81; #28, #41 |
| `eg.from_edges` ‡ | A graph from two equal-length arrays of identifiers (NumPy or Arrow), with the same `self_loops` and `duplicates` as `eg.read`. Kept in memory below 100 MB of edges †; above, spilled to a hidden `.eg` in the scratch directory, removed when the graph object is released unless `g.save()` keeps it. | #84 |
| `eg.from_scipy` ‡ | A graph from a SciPy sparse matrix; `directed=None` means directed unless the matrix is symmetric. | #84 |

### 3.2 `Graph`

A graph is immutable. It is either backed by a `.eg` file — opening it is instant and it
may be larger than memory — or held in memory. Nodes have identifiers; edges may carry a
weight, a time and other columns; repeated edges between the same two nodes are separate
events unless the graph was read with `duplicates="collapse"` or `"sum"`
(`docs/spec/graph-semantics.md`, written by #36, defines what each algorithm does with
them).

```python
class Graph:
    num_nodes: int                    # on a view: counted on first use (§ 3.3)
    num_edges: int                    # stored edges: every event counts
    directed: bool
    file: pathlib.Path | None         # None for a graph in memory
    columns: Table                    # name, on, type, role

    def info(self) -> Info: ...
    def nodes(self) -> Table: ...
    def edges(self) -> Table: ...
    def save(self, path: str | os.PathLike, *, overwrite: bool = False,
             **run: Unpack[Run]) -> pathlib.Path: ...              # ‡ #84

    # --- questions about the whole graph ---------------------------------
    def pagerank(self, *, damping: float = 0.85, tol: float = 1e-6,
                 max_iter: int = 100, weight: bool | str = True,
                 personalize: Nodes | None = None,
                 precision: Literal["full", "low"] = "full",
                 **run: Unpack[Run]) -> Table: ...
    def components(self, *, strong: bool = False,
                   **run: Unpack[Run]) -> Table: ...
    def kcore(self, **run: Unpack[Run]) -> Table: ...
    def triangles(self, **run: Unpack[Run]) -> Table: ...
    def degrees(self, nodes: Nodes | None = None, *,
                weight: bool | str = True,
                **run: Unpack[Run]) -> Table: ...
    def bfs(self, source: Node, *, direction: Direction = "out",
            max_hops: int | None = None, **run: Unpack[Run]) -> Table: ...
    def top(self, by: Measure, n: int = 10,
            **run: Unpack[Run]) -> Table: ...
    def communities(self, *, method: Literal["leiden"] = "leiden",
                    resolution: float = 1.0,
                    objective: Literal["modularity", "cpm"] = "modularity",
                    iterations: int = 1,
                    weight: bool | str = True,
                    use_direction: bool = False,
                    deterministic: bool = True, seed: int = 0,
                    **run: Unpack[Run]) -> Communities: ...
    def map(self, *, communities: Communities | None = None,
            deterministic: bool = True, seed: int = 0,
            **run: Unpack[Run]) -> Map: ...
    def explain(self, operation: str, /, **parameters) -> Plan: ...

    # --- questions about particular nodes --------------------------------
    def neighbors(self, nodes: Nodes, *, direction: Direction = "out",
                  limit: int = 100_000) -> Table: ...
    def khop(self, node: Node, *, k: int, direction: Direction = "out",
             limit: int = 100_000, timeout: float = 10.0) -> Table: ...
    def path(self, source: Node, target: Node, *,
             max_hops: int | None = None, avoid: Nodes | None = None,
             weight: str | None = None,
             limit: int = 1_000_000, timeout: float = 10.0) -> Path: ...
    def has_edge(self, source: Node, target: Node) -> bool: ...    # ‡ #90
    def common_neighbors(self, a: Node, b: Node, *,
                         limit: int = 100_000) -> Table: ...       # ‡ #90
    def edges_between(self, a: Nodes, b: Nodes, *,
                      limit: int = 100_000) -> Table: ...          # ‡ #90

    # --- part of the graph ------------------------------------------------
    def filter(self, *, edges: str | None = None,
               nodes: str | None = None) -> View: ...
    def window(self, start: Time | None = None,
               end: Time | None = None) -> View: ...
    def subgraph(self, nodes: Nodes) -> View: ...

    # --- to small-graph tools ---------------------------------------------
    def to_networkx(self, *, max_edges: int = 1_000_000): ...      # ‡ #85
    def to_igraph(self, *, max_edges: int = 1_000_000): ...        # ‡ #85
    def to_scipy(self, *, weight: bool | str = True): ...          # ‡ #85
```

The values of `limit`, `timeout`, `max_edges`, `tol`, `max_iter` and `iterations` are †.

**The whole graph.** Each of these is planned, prints its plan and one progress line
when long, is deterministic (§ 2.11), and returns one row per node of the graph or
view, in graph order (§ 2.12). On the whole graph of a file the result is stored in the
file and returned from there the next time; on a view it is kept for the session only
(§ 2.8).

| Method | Returns (columns) | Notes | Implemented by |
|---|---|---|---|
| `pagerank()` | `node`, `pagerank` | Scores sum to 1. Stops when the sum of absolute changes between two iterations is below `tol` — not scaled by the number of nodes, unlike NetworkX — or after `max_iter` iterations; `meta` has `iterations` and `converged`. `personalize`: restart at these nodes only. `precision="low"`: 32-bit state, labelled, never chosen for you (§ 2.4). | #47 |
| `components()` | `node`, `component` | Weakly connected components; `strong=True` for strongly connected ones (directed graphs only). Numbered from 0 by decreasing size (§ 2.12). `.sizes()` gives `component`, `nodes`. | #48 |
| `kcore()` | `node`, `core` | The core number of each node, direction ignored (§ 2.13). `.sizes()` gives `core`, `nodes`. | #50 |
| `triangles()` | `node`, `triangles`, `clustering` | Triangles through each node and its local clustering coefficient. `meta` has `triangles_total` and `transitivity`. Direction is ignored. | #51 |
| `degrees(nodes=None)` | `node`, `in_degree`, `out_degree`, `degree`, `weighted` | For every node, or for the nodes given. Each counts **distinct neighbors**, however many edges join the pair (a pair is analysed once, `choices.md` A1): `in_degree` the nodes with an edge to it, `out_degree` the nodes it has an edge to, `degree` the nodes joined to it in either direction — so `degree` can be less than `in_degree + out_degree`. Undirected graphs have `degree` only. `weighted`, present when the graph has a weight: the sum of the weight (§ 2.4) over its pairs. On the whole graph they are made with the graph and cost nothing (#50); on a view they are the view's, computed by a planned pass. | #50, #90 |
| `bfs(source)` | `node`, `distance`, `parent` | Hops from `source` to every node; null where unreachable. `max_hops` stops early. `parent`: § 2.12. Kept for the session only (§ 2.8). | #49 |
| `top(by, n)` | `node`, the measure, `rank` | The `n` nodes with the highest value of a per-node measure: `"degree"`, `"in_degree"`, `"out_degree"`, `"weighted"`, `"pagerank"`, `"core"`, `"triangles"`, `"clustering"`. Computes the measure with default parameters if it is not stored. Ties and ranks: § 2.12. Same as `g.pagerank().top(n)` and so on. | #81 |
| `communities()` | a `Communities` (§ 3.5) | `method`: `"leiden"`, the only method in 0.1; the parameter is where other methods will go. `resolution`: larger gives more, smaller communities. `objective`: `"modularity"` or `"cpm"` (#58). `iterations`: 1 by default, as #57 specifies, to be confirmed by #57's measurements (`design/DECISIONS.md`, "Default number of Leiden iterations"). `weight`: § 2.4; the summary says which was used. `use_direction=False`: direction is ignored when finding communities, and the result says so (`choices.md` A2); `True` uses it (#58), and is `Unsupported` on an undirected graph. | #57, #58, #60, #62 |
| `map()` | a `Map` (§ 3.8) | Does everything that is missing — communities, hierarchy, layout, tiles — after printing the plan, and writes the working map `NAME.egmap` beside the graph (a view's or a graph in memory's is kept in scratch until exported). `communities=`: draw these instead of the default ones. | #63, #67, #69, #76 |
| `explain(operation, **parameters)` | a `Plan` (§ 3.9) | `g.explain("communities", memory="4GB")` is `g.communities(memory="4GB", dry_run=True)`. `operation` is the name of any method that takes `**run`. Never raises a refusal: a plan that does not fit says so. | #45 |

**Particular nodes.** These are answered at once: no plan, no progress, nothing stored.
Each takes a `limit`, and where a search could run long a `timeout`; reaching either
returns what was found, marked `truncated` (§ 2.5).

| Method | Returns (columns) | Notes | Implemented by |
|---|---|---|---|
| `neighbors(nodes)` | `neighbor`, `edges`, then the weight column under its own name, `first`, `last` | One row per distinct neighbor: how many edges join the pair, the sum of their weight, the first and last time. The weight is present when the graph has a weight column, and `first` and `last` when it has a time column. With several nodes, a leading `node` column says whose neighbor each row is. Order, and `limit` per node: § 2.12. | #90 |
| `khop(node, k=…)` | `node`, `distance` | Every node within `k` hops, nearest first. `limit` counts nodes returned; which are kept: § 2.12. | #90 |
| `path(source, target)` | a `Path` (§ 3.7) | A shortest path by number of hops; with `weight="column"`, by the smallest sum of that column (a cost — this is not the `weight` of § 2.4). `max_hops`: give up beyond this length. `avoid`: nodes the path may not pass through. `limit` counts nodes the search may visit. On a view, the path stays inside the view. | #91, #38 |
| `has_edge(source, target)` ‡ | `bool` | | #90 |
| `common_neighbors(a, b)` ‡ | `neighbor`, `edges_a`, `edges_b` | | #90 |
| `edges_between(a, b)` ‡ | `source`, `target`, `edges`, weight, `first`, `last` | Pairs joined by at least one edge, one end in each set. | #90 |

**Out of the graph.**

| Member | Returns | Notes | Implemented by |
|---|---|---|---|
| `num_nodes`, `num_edges`, `directed`, `file` | | On a view, the counts are the view's: computed on first access by one planned pass, then kept (§ 3.3). | #81, #93 |
| `columns` | `name`, `on`, `type`, `role` | The columns of the graph: `on` is `nodes` or `edges`; `role` is `source`, `target`, `weight`, `time` or empty. These are the names a filter may use, with the degree names. | #37 |
| `info()` | an `Info` | A mapping whose text form is exactly what `everygraph info` prints (below): the file, its counts and columns, and everything computed so far with its parameters, time and date. | #39, #98 |
| `nodes()` | `node`, then node columns | One row per node, in graph order (§ 2.2). | #81, #83 |
| `edges()` | `source`, `target`, then edge columns | One row per stored edge. Streamed: it is as large as the graph. | #81, #83 |
| `save(path)` ‡ | the path | Writes the graph or view as a new file; the format follows the extension: `.eg`; `.parquet`, `.csv` (the edge table); `.graphml`, `.gexf`, `.mtx`. Planned like any long operation. An existing file: § 2.7. | #84, #85 |
| `to_networkx()` ‡ | `networkx.Graph` or `DiGraph` | Each connected pair becomes one edge with attributes `edges` (how many) and the summed weight. Nodes are the original identifiers and carry `community` and the map position `x`, `y` when those are computed. Above `max_edges` it raises `Unsupported` with the ways forward (story 7). | #85, #87 |
| `to_igraph()`, `to_scipy()` ‡ | an `igraph.Graph`; a SciPy sparse matrix | Same guard and same projection. | #85 |

**What it prints.** A graph in a file prints the first block of `everygraph info`;
`g.info()` prints all of it. A graph in memory prints as in story 10. The notebook card
is "the graph card" of `notebook.html` (name, kind, the two counts with the user's
nouns). Where a line has no room for the noun, it is dropped (`31,960` alone after
`communities (…)`, `design/DECISIONS.md`, "The word for what community detection
finds").

```text
>>> g.info()
transfers.eg · 9.3 GB · created 2026-11-24 from transfers.parquet

  nodes       212,418,907
  edges     1,830,397,917 · directed · duplicates kept as events
  ids       hex addresses
  time      2024-01-01 … 2024-12-31
  columns   value (weight), block_time (time)

  computed so far
    degrees                              at ingest
    communities (Leiden, deterministic)  31,960 · 2 min 11 s · 2026-11-24
    pagerank                             41 s · 2026-11-24
    map                                  transfers.egmap · 4.1 GB
```

### 3.3 `View`

A view is part of a graph. It is a `Graph` — `isinstance(view, eg.Graph)` holds, and
every method of § 3.2 works on it — defined by text, so making one reads nothing and it
can be stored by its definition. Whether it is evaluated through a mask or materialized
as a smaller graph is the planner's choice (`docs/DESIGN.md` D10); the result is the
same either way (§ 2.11).

```python
class View(Graph):
    base: Graph                       # the graph it is part of
    definition: dict                  # window, edges, nodes: what is printed

    def __and__(self, other: View) -> View: ...
```

| Made by | Keeps | Notes | Implemented by |
|---|---|---|---|
| `g.filter(edges="value > 10")` | The edges for which the expression is true, and the nodes that still have an edge. | The expression language is below. | #93, #37 |
| `g.filter(nodes="in_degree > 100")` | The nodes for which it is true, and the edges with both ends among them. | Names available in 0.1: the graph's node columns, and `in_degree`, `out_degree`, `degree` — the degrees of the graph or view the filter is applied to. | #93 |
| `g.window(start, end)` | The edges whose time is from `start` to `end`, both included, and the nodes that still have an edge. | Times as in § 2.3: a date means its whole day. Either bound may be `None`. Needs a time column and a graph not read with `duplicates="sum"`, else `Unsupported` (§ 2.13). | #94 |
| `g.subgraph(nodes)` | The given nodes and the edges with both ends among them. | Accepts any node set (§ 2.2): `g.subgraph(c[0])`, `g.subgraph(m.selection)`, `g.subgraph(g.khop(a, k=2))`. | #87, #93 |
| `a & b` | What is in both. | Both must be views of the same graph, and each part keeps its meaning on that graph. A chain (`g.window(…).filter(…)`) applies each part to the view before it: the same thing as `&`, except that a node filter on degrees reads the degrees of the view it is applied to. | #93 |

**Counts.** A view's `num_nodes` and `num_edges` are not known when it is made. The
first access to either computes both with one planned pass — the phase `counting`,
with its plan and progress line when it is long (`docs/ux-contract.md` P-3), using the
memory and threads the base graph was opened with — and keeps them with the view. Its text form shows them only once known
(story 4). `degrees()` on a view is a planned operation of the same kind and takes
`**run`. What needs a view's counts (the guard of `to_networkx`) gets them the same way,
or from a result that already holds them: `g.subgraph(c[4])` has the `nodes` and
`internal_edges` of `c.table()`.

A filter is short text (`api.md` Choice 9), checked when `filter()` is called:

```text
expression := term { ("and" | "or") term }        "and" binds tighter
term       := ["not"] ( comparison | "(" expression ")" )
comparison := column op value
            | column ["not"] "in" "(" value { "," value } ")"
            | column "is" ["not"] "null"
op         := "==" | "!=" | "<" | "<=" | ">" | ">="
column     := a name from g.columns or a degree name,
              or `any name in backquotes`
value      := a number | 'text' | "text" | true | false
```

The words `and`, `or`, `not`, `in`, `is`, `null`, `true` and `false` are reserved: a
column with one of these names is written in backquotes (`` `in` ``). A time column is
compared with text in ISO 8601 form (`block_time >= '2024-03-01'`, with the meaning of
§ 2.3) or, if it holds integers, with a number. There is no arithmetic and there are no
functions in 0.1. A mistake raises `InvalidExpression`, whose text shows the expression
with a caret under the place.

A view prints as in story 4: one headline — with its counts once they are known, `not
counted yet` before — then one line for each part of its definition. Printing never
starts the count. Results computed on a view say `(view)` after the graph's name.

### 3.4 `Table`

Every algorithm and query returns a table. It **is** an Arrow table: it implements the
Arrow PyCapsule interface, so `pl.from_arrow(t)`, `pa.table(t)`, `t.to_pandas()` and
`duckdb.sql("select … from t")` read it directly, the numeric columns without a copy
(#83, #95). It is a handle on a stored result, so a table with a row for every node of a
large graph costs no memory until it is handed over, and then it is streamed in batches.
The identifier column is produced as the table is handed over and costs what the
identifiers weigh.

```python
class Table:
    columns: list[str]
    num_rows: int
    truncated: bool
    meta: Mapping[str, object]

    def head(self, n: int = 5) -> Table: ...
    def top(self, n: int = 10, *, by: str | None = None) -> Table: ...
    def sizes(self) -> Table: ...
    def on_map(self) -> Map: ...
    def write(self, path: str | os.PathLike, *, overwrite: bool = False,
              **run: Unpack[Run]) -> pathlib.Path: ...
    def to_arrow(self) -> "pyarrow.Table": ...
    def to_pandas(self) -> "pandas.DataFrame": ...

    def __len__(self) -> int: ...
    def __arrow_c_schema__(self) -> object: ...
    def __arrow_c_stream__(self, requested_schema: object | None = None
                           ) -> object: ...
```

| Member | Returns | Notes | Implemented by |
|---|---|---|---|
| `columns`, `num_rows`, `len(t)` | | | #81 |
| `truncated` | `bool` | § 2.5. | #90 |
| `meta` | a read-only mapping | Always: `operation`, `parameters`, `graph`, `view`, `engine`, `deterministic`, `seed`, `precision`, `weight`, `direction`, `took_seconds`, `peak_memory_bytes`, `cached`, `plan`, `truncated`. Plus what the operation adds (`iterations`, `modularity`, `triangles_total`, …). Exported as the schema metadata of the Arrow stream under the key `everygraph`, which is also how a Parquet or Arrow file written by Everygraph is recognized (§ 2.7). | #45, #39 |
| `head(n)` | `Table` | The first `n` rows. | #81 |
| `top(n, by=…)` | `Table` | The `n` rows with the largest value of `by` (default: the table's last numeric column), largest first, with a `rank` column added; ties and ranks as in § 2.12. `by` is keyword-only, so `pr.top(5)` and `g.top("degree", 5)` cannot be confused. | #47, #81 |
| `sizes()` | `Table` | For a table that assigns each node to a component, a core or a community: one row per value with its number of nodes, largest first (§ 2.12). | #48, #50 |
| `on_map()` | `Map` | Shows these nodes on the graph's map: `g.map().highlight(t)`, and returns the map. Builds the map if it does not exist, after printing the plan. | #76 |
| `write(path)` | the path | Format from the extension: `.parquet`, `.csv`, `.tsv`, `.arrow`. The Python form of `--out`. Streams the whole table; planned when long. An existing file: § 2.7. | #83, #85 |
| `to_arrow()`, `to_pandas()` | | Conveniences for code that expects those objects (Polars and DuckDB users often call `to_arrow()`); they need `pyarrow`, respectively `pandas`, installed, and neither is a dependency of `everygraph`. | #83 |

The columns of every table, which are part of the API. `uint32` is 32-bit in 0.1 and its
width is not frozen (§ 2.3); identifier columns — `node`, `neighbor`, the `parent` of
`bfs()`, `source`, `target` — have the type of the input's identifiers (§ 2.2).

| From | Columns and types |
|---|---|
| `pagerank()` | `node` · `pagerank` float64 (float32 with `precision="low"`) |
| `components()` | `node` · `component` uint32 |
| `kcore()` | `node` · `core` uint32 |
| `triangles()` | `node` · `triangles` uint64 · `clustering` float64 |
| `degrees()` | `node` · `in_degree` uint32 · `out_degree` uint32 · `degree` uint32 · [`weighted` float64 or decimal] |
| `bfs()` | `node` · `distance` uint32, nullable · `parent` identifier, nullable |
| `top()`, `.top()` | the source table's columns · `rank` uint32 |
| `.sizes()` | the column it groups by · `nodes` uint32 |
| `neighbors()` | [`node`] · `neighbor` · `edges` uint64 · [weight under its name] · [`first`, `last` timestamp] |
| `khop()` | `node` · `distance` uint32 |
| `common_neighbors()` | `neighbor` · `edges_a` uint64 · `edges_b` uint64 |
| `edges_between()` | `source` · `target` · `edges` uint64 · [weight under its name] · [`first`, `last` timestamp] |
| `c.nodes()` | `node` · `community` uint32 |
| `c.table()` | `community` uint32 · `nodes` uint32 · `internal_edges` uint64 · `external_edges` uint64 · `share` float64 · [`parent` uint32] |
| `c.of()` | [`node`] · `level` uint32 · `community` uint32 · `nodes` uint32 |
| `p.nodes()` | `step` uint32 · `node` |
| `p.edges()` | `step` uint32 · `source` · `target` · `edges` uint64 · [weight under its name] · [`time` timestamp] |
| `m.selection.nodes()` | `node` · `community` uint32 · `in_degree` · `out_degree` · `degree` uint32 |
| `m.hierarchy.table()` | `level` · `region` uint32 · `parent` uint32, nullable · `kind` text · `name` text, nullable · `nodes` uint32 · `internal_edges` · `external_edges` uint64 |
| `g.nodes()` | `node` · node columns |
| `g.edges()` | `source` · `target` · edge columns |
| `g.columns` | `name` · `on` · `type` · `role`, all text |

`internal_edges` counts the stored edges (events, like `num_edges`) with both ends in
the community or region; `external_edges` those with exactly one end in it, in either
direction — on a directed graph, both those leaving and those arriving. Neither counts
pairs. `share` is a fraction from 0 to 1 of all nodes in the data, and a percentage when
printed. A weight that was a decimal or an integer in the input stays exact (#36).

The text form is described in § 2.6 and shown in the stories; the notebook form is the
table card of `notebook.html` (cells 5, 6 and 11), with the same footer.

### 3.5 `Communities` and `Community`

What `g.communities()` returns: the partition of the nodes into communities, the coarser
and finer levels the method went through, and exact counts for every community at every
level.

```python
class Communities:
    modularity: float                 # 0 when there are no edges
    levels: int
    meta: Mapping[str, object]

    def table(self, level: int = 1) -> Table: ...
    def nodes(self, level: int = 1) -> Table: ...
    def of(self, nodes: Nodes) -> Table: ...
    def __getitem__(self, community: int) -> Community: ...
    def __len__(self) -> int: ...                 # number of communities
    def __arrow_c_schema__(self) -> object: ...   # the same table as nodes()
    def __arrow_c_stream__(self, requested_schema: object | None = None
                           ) -> object: ...

class Community:
    id: int
    def nodes(self) -> Table: ...                 # node
    def __len__(self) -> int: ...                 # number of nodes
```

| Member | Returns | Notes | Implemented by |
|---|---|---|---|
| `len(c)`, `c.modularity`, `c.levels` | | The number of communities in the partition, its modularity, and how many levels there are. | #57, #62 |
| `c.table(level=1)` | one row per community | Level 1 is the partition — the answer. Level 2 and beyond are the finer levels the method built it from, each row with its `parent` at the level above. Communities are numbered from 0 by decreasing size at every level, so `community` 0 is the largest; ties: § 2.12. Counts are exact at every level (§ 3.4 for what they count). | #62 |
| `c.nodes(level=1)` | `node`, `community` | The community of every node. Handing `c` itself to Polars or DuckDB gives this table. | #57, #62, #83 |
| `c.of(nodes)` | `level`, `community`, `nodes` | Which community a node is in at every level, and how large that community is. For several nodes, a leading `node` column. | #62 |
| `c[i]` | a `Community` | Community `i` of the partition, usable wherever a node set is expected: `g.subgraph(c[0])`, `avoid=c[3]`, `m.highlight(c[0])`. `Community.id` is `i`. | #62, #87 |

It prints as approved in `api.md` § 3 (stories 1 and 4). The lines, in order: what and
how (`Leiden, resolution 1.0, deterministic` — or `fast`); the number of communities,
the modularity, the number of disconnected communities (always 0 for Leiden; it is
printed because it is the guarantee); `levels`, the size of each level from the nodes to
the partition — the reverse of the numbering (§ 2.12); the largest, the median and the
singletons; `weight`, which weight was used (`value, summed per pair`, `edges per pair
(no weight column was declared)`, or `none` with `weight=False`); a `note` line when
direction was ignored on a directed graph; the time taken. The notebook card is cell 4
of `notebook.html`.

### 3.6 `Hierarchy`

The tree a map is drawn from (`docs/DESIGN.md` D16). It is not the same thing as the
levels of `Communities`: it is shaped for reading — no region has too many children —
and so it contains three kinds of region, which the map draws differently and never
confuses (`SPEC.md`, "The map"):

| `kind` | What it is | On the map |
|---|---|---|
| `community` | A community found by the algorithm. | solid outline |
| `subdivision` | A split of one community, made only so the map stays readable. | dashed outline |
| `aggregate` | Many small communities shown as one. | hatched |

```python
class Hierarchy:
    levels: int
    def table(self, level: int | None = None) -> Table: ...
    def of(self, nodes: Nodes) -> Table: ...
```

| Member | Returns | Notes | Implemented by |
|---|---|---|---|
| `levels` | `int` | The number of zoom levels; level 1 is the top. | #63 |
| `table(level=None)` | one row per region | All levels, or one. `region` numbers are unique within the hierarchy; `parent` is null at level 1; `name` is the region's label on the map, or null. Counts are exact. The children of a region are the rows whose `parent` is that region. | #62, #63 |
| `of(nodes)` | `level`, `region`, `kind`, `name`, `nodes` | The regions containing a node, from the top down. | #62 |

### 3.7 `Path`

```python
class Path:
    found: bool
    hops: int | None                  # None when not found
    truncated: bool                   # the search stopped at limit or timeout
    meta: Mapping[str, object]

    def nodes(self) -> Table: ...     # step, node
    def edges(self) -> Table: ...     # step, source, target, edges, weight,
                                      # time
    def alternatives(self, *, limit: int = 10) -> list[Path]: ...
    def on_map(self) -> Map: ...
```

| Member | Notes | Implemented by |
|---|---|---|
| `found`, `hops` | No path within the constraints is a result, not an error: `found` is `False`. If the search was cut short as well, a path may still exist, and the text says so. | #91 |
| `nodes()`, `edges()` | The path as tables. `edges()` has one row per step: the pair, how many edges join it inside the view (`edges`), and the weight and time of the one edge the step shows (§ 2.12). | #91 |
| `alternatives(limit=…)` | The other shortest paths of the same length, at most `limit` †, in the order #91 defines. The only place a Python list is returned; it is short by construction. | #91 |
| `on_map()` | Draws the path on the graph's map, dimming the rest. | #76 |
| `meta` | `avoided` (how many nodes were excluded), `alternatives` (how many other shortest paths exist, counted up to a bound), `visited`, `took_seconds`. | #91 |

Text form, approved in `api.md` § 4 (story 3): `Path`, the hops, the time; one line per
step with the weight and the time of the earliest edge of the pair inside the view, and
`· N events` after them when the pair has more than one (an event is one stored edge);
a last line with what was avoided and how many alternatives exist. The weight is
labelled with its column's name. When nothing is found:

```text
>>> g.path("0x4f3a…91bc", "0x9e12…07aa", max_hops=2)
Path · none within 2 hops · searched in 0.4 ms
  from 0x4f3a…91bc to 0x9e12…07aa
  to look further: g.path("0x4f3a…91bc", "0x9e12…07aa", max_hops=5)
```

### 3.8 `Map` and `Selection`

```python
class Map:
    file: pathlib.Path                # the working .egmap
    levels: int                       # zoom levels
    hierarchy: Hierarchy
    selection: Selection

    def export(self, path: str | os.PathLike, *,
               reveal: Literal["full", "summary"],
               overwrite: bool = False,
               **run: Unpack[Run]) -> pathlib.Path: ...
    def highlight(self, nodes: Nodes | None) -> None: ...

class Selection:
    kind: Literal["none", "node", "region"]
    def nodes(self) -> Table: ...
```

| Member | Notes | Implemented by |
|---|---|---|
| `g.map()` | § 3.2. The working map is `NAME.egmap` beside the graph, and it is full: every identifier, for this computer (a view or a graph in memory keeps it in scratch until exported). Calling it again returns the same map without rebuilding. | #63, #67, #69 |
| displaying `m` | In a notebook: the interactive map, the same viewer as `everygraph open`, with its caption strip. At a prompt: the text form of story 5. | #76, #71, #78 |
| `m.export(path, reveal=…)` | Writes a map file for sharing, with `viewer.html` beside it: one static file to host anywhere. **`reveal` is required and has no default** (`design/DECISIONS.md`, "What a shared map reveals"): `"full"`, every node identifier and the links shown; `"summary"`, regions and counts only, no individual identifiers (`choices.md` C3). A map never contains more than it shows. Leaving `reveal` out raises a `TypeError` whose message gives the two ways forward (below). | #75 |
| `m.highlight(nodes)` | Marks these nodes on the displayed map; `None` clears. A table's other columns are not used in 0.1. | #76 |
| `m.selection` | What is selected on the displayed map right now. `kind` is `"none"` before anything is clicked. | #76 |
| `m.selection.nodes()` | The selected nodes as a table (story 5): the node itself, or every node of the selected region, ordered as in § 2.12. `community` is the community of the partition (level 1 of `Communities`) each node belongs to: for a region of kind `community`, that community for every row; for a `subdivision`, the community it is a part of; for an `aggregate`, each node's own small community, so the rows carry several numbers. The degree columns are those of the graph the map was made from. Nothing selected gives a table with the same columns and no rows, whose footer says how to select — not an error. A `Selection` is a node set (§ 2.2). | #76 |
| `m.hierarchy` | § 3.6. | #63 |
| `m.file`, `m.levels` | | #69 |

```text
>>> m.export("shared.egmap")
Traceback (most recent call last):
  File "<stdin>", line 1, in <module>
TypeError: export() needs reveal=: a shared map holds either every node
identifier or only regions and counts, and nothing is chosen for you.

  Ways forward:
    m.export("shared.egmap", reveal="summary")    regions and counts only
    m.export("shared.egmap", reveal="full")       every identifier shown

Nothing was written.
```

### 3.9 `Plan`

What `g.explain(...)` and `dry_run=True` return, and what is printed before every long
operation. Its fields, its text forms, its JSON schema and the refusal form are specified
in `docs/ux-contract.md` § 3.

```python
class Plan:
    operation: str
    fits: bool
    estimate: Mapping[str, object]    # seconds, peak_memory_bytes,
                                      # scratch_bytes, output_bytes
    refusal: Mapping[str, object] | None

    def to_dict(self) -> dict: ...    # the document of ux-contract § 3.6
```

A plan is a value: making one runs nothing and writes nothing. A plan that does not fit
is still returned — `fits` is `False` and `refusal` holds what the exception would have
said. It prints as in story 8; its notebook form is the plan card (cell 3 of
`notebook.html`).

### 3.10 Exceptions and warnings

```python
class Way:                                   # one way forward
    command: str                             # as the command line shows it
    python: str                              # the same, as a call
    note: str                                # its consequence, or ""
    needs_bytes: int | None

class EverygraphError(Exception):
    message: str
    why: str | None
    arithmetic: Mapping[str, object] | None  # count, unit, bytes_each
    ways_forward: list[Way]
    changed: bool
    kept: Mapping[str, object] | None        # phases, path, bytes (C-8)
    machine_memory_bytes: int | None
    def to_dict(self) -> dict: ...

class InsufficientBudget(EverygraphError): ...
class InsufficientDisk(EverygraphError): ...
class InvalidInput(EverygraphError): ...
class InvalidExpression(EverygraphError, ValueError): ...
class UnknownNode(EverygraphError, LookupError): ...
class Unsupported(EverygraphError): ...
class OutputExists(EverygraphError, FileExistsError): ...
class CorruptFile(EverygraphError): ...

class TruncatedWarning(UserWarning): ...     # a result was cut short (§ 2.5)
```

When each is raised, their attributes and their exit statuses are in § 2.7; the message
pattern is in `docs/ux-contract.md` § 6.1, whose error document `to_dict()` returns,
`ways_forward` as a list of objects with the keys of `Way`. In a notebook an exception is shown as the
error card, explanation first and traceback folded (#76). `TruncatedWarning` is issued
with `warnings.warn`, so Python displays it as it displays any warning (story 3) and
its filters apply; in a notebook the result's card also carries the "Cut short." note
of `notebook.html` cell 8, so the warning reaches loops and scripts that never display
the result, and the card the person looking at it.

---

## 4. The Rust embedding API

For the developer who embeds a graph or a map in an application (`docs/DESIGN.md` § 2).
The Python package and the command are thin layers over these crates, so this section
mirrors § 3 name for name: free functions instead of methods, a struct of parameters
instead of keywords, `Result` instead of exceptions.

These are signatures, not code: the workspace exists (#2), and the crates' internals
belong to their own issues. Lifetimes, generic bounds and the exact module paths are
indicative; the names, the shape of the parameters and results, and which crate owns
what are the specification. The Rust API is unstable in 0.1 (§ 2.10).

```rust
// ---- everygraph-core: shared types, no file-system concepts (D2) ----------

pub struct NodeId(pub u32);        // dense: 0..num_nodes (D3); not in Python
pub type EdgeOffset = u64;

pub enum Direction { Out, In, Both }
pub enum Precision { Full, Low }                     // PageRank only, in 0.1
pub enum Weight<'a> { Declared, Column(&'a str), None }   // True, "name", False
pub enum Measure {                                   // g.top(by, n)
    Degree, InDegree, OutDegree, Weighted,
    PageRank, Core, Triangles, Clustering,
}

/// What every planned operation takes: the `**run` of the Python API.
/// `Run::default()` gives the documented defaults and discards events.
pub struct Run<'a> {
    pub memory: Option<u64>,             // bytes; None: half of free memory
    pub threads: Option<NonZeroUsize>,   // None: the planner chooses
    pub scratch: Option<&'a dyn Scratch>,// None: the caller's default (§ 2.4)
    pub events: &'a dyn Events,          // plan, progress, warnings
    pub cancel: &'a Cancel,              // checked at chunk boundaries (C-4)
}

/// One job's scratch, already resolved for it by the caller: the hidden
/// directory .everygraph-ID from everygraph-io (`DirScratch`), or memory in a
/// browser. Finding a job's directory by its fingerprint, resuming, and
/// listing other jobs' directories (ux-contract R-5, R-7, R-10) are done above
/// the trait, in everygraph-io.
///
/// Today the trait is in everygraph-ingest (`ids::scratch`, #34), the only
/// crate that uses it, and this is its shape. It moves to everygraph-core when
/// a second crate needs it; #35 decides then what it gains (reopening what a
/// stopped run kept, the space left, whether it counts against memory, a name
/// for messages) and how `Run::scratch` holds it.
pub trait Scratch: Sync {
    type Source: ByteSource + Send + Sync;       // what a sealed writer reads as
    type Writer: Write + Send;                   // append-only
    fn create(&self, purpose: &'static str) -> io::Result<Self::Writer>;
    fn seal(&self, writer: Self::Writer) -> io::Result<Self::Source>;
}

pub trait Events: Sync {
    fn event(&self, event: &Event);      // never called concurrently
}
pub enum Event {                         // ux-contract § 4.6, one to one
    Plan(Plan), Phase(Phase), Warning(Warning),
    Result(Report), Stopped(Stopped), Error(ErrorDoc),
}

/// Why a run was stopped: the `reason` of the stop document.
pub enum StopReason { Interrupt, Terminate, Exception }
pub struct Cancel { /* an atomic flag and the first reason given */ }
impl Cancel {
    pub fn new() -> Cancel;
    pub fn cancel(&self, reason: StopReason);   // Ctrl-C: Interrupt
    pub fn reason(&self) -> Option<StopReason>; // None: not cancelled
}

/// A clean stop is not an error (ux-contract § 5.4): a planned function
/// returns its result or `Stopped`, once its workers have stopped and its
/// scratch is tidied.
pub enum Outcome<T> {
    Done(T),
    Stopped(Stopped),
}
pub struct Stopped { /* the stop object of ux-contract § 4.6 */ }
impl Stopped {
    pub fn reason(&self) -> StopReason;
    pub fn to_json(&self) -> String;     // the stop document, ux-contract § 5.4
}
pub type Planned<T> = Result<Outcome<T>>;

pub struct Plan { /* the fields of ux-contract § 3.2 */ }
impl Plan {
    pub fn fits(&self) -> bool;
    pub fn refusal(&self) -> Option<&Refusal>;
    pub fn to_json(&self) -> String;     // the schema of ux-contract § 3.6
}
impl fmt::Display for Plan { /* the text form */ }

pub enum Error {                         // § 2.7
    InsufficientBudget(Refusal),
    InsufficientDisk(Refusal),           // names the scratch directory
    InvalidInput(InvalidInput),          // rows, or a weight column (§ 2.13)
    InvalidExpression { expression: String, position: usize, why: String },
    UnknownNode { node: String },
    Unsupported { operation: String, reason: String },
    Storage(Box<dyn core::error::Error + Send + Sync>),
                                         // OutputExists, CorruptFile and I/O
                                         // errors of -io and -format, boxed
}
impl Error {
    pub fn ways_forward(&self) -> &[Way];
    pub fn changed(&self) -> bool;
    pub fn to_json(&self) -> String;     // the schema of ux-contract § 6.1
}
pub type Result<T> = core::result::Result<T, Error>;

/// Implemented by `Graph` and by `View`: anything an algorithm can run on.
pub trait GraphRef {
    fn num_nodes(&self, run: &Run) -> Planned<u64>;  // a pass on a view (§ 3.3)
    fn num_edges(&self, run: &Run) -> Planned<u64>;
    fn is_directed(&self) -> bool;
    fn columns(&self) -> &[ColumnInfo];
    fn resolve(&self, ids: &Ids) -> Vec<Option<NodeId>>;   // in bulk (#34)
    fn id_of(&self, node: NodeId) -> Id<'_>;
}

/// A result. Columns are Arrow-compatible buffers; `everygraph-py` exports
/// them through the Arrow C interface without copying (#83).
pub struct Table { /* columns, meta */ }
impl Table {
    pub fn columns(&self) -> &[Column];
    pub fn num_rows(&self) -> u64;
    pub fn column<T: Value>(&self, name: &str) -> Option<ColumnRef<'_, T>>;
    pub fn meta(&self) -> &Meta;         // parameters, plan, took, cached …
    pub fn truncated(&self) -> Option<&Truncated>;
    pub fn top(&self, n: usize, by: Option<&str>) -> Table;   // a column name
    pub fn head(&self, n: usize) -> Table;
}

// ---- everygraph-io: files, scratch directories, their errors (#29) --------

/// The scratch root when none is given (ux-contract § 1): the output's
/// directory, else the graph file's, else the system temporary directory.
pub fn scratch_root(output: Option<&Path>, graph: Option<&Path>) -> PathBuf;
pub struct DirScratch { /* one job's directory .everygraph-ID */ }
impl DirScratch {
    /// The directory of this job in the scratch root: the one a stopped run
    /// left, checked against its manifest (R-5, R-7), or a new one.
    pub fn for_job(root: &Path, job: &Fingerprint) -> io::Result<DirScratch>;
    pub fn kept(&self) -> &[KeptPhase];           // empty for a new job
    /// Other jobs' directories in the root with no live owner (R-10).
    pub fn others(root: &Path) -> io::Result<Vec<Manifest>>;
}
impl Scratch for DirScratch { /* … */ }
pub struct IoError { pub path: PathBuf, pub source: io::Error }
pub struct OutputExists { pub path: PathBuf, pub holds: Holds }
pub enum Holds { OtherGraph, NotOurs }

// ---- everygraph-format: the graph file (#28, #39, #41) --------------------

pub struct CorruptFile { pub path: PathBuf, pub section: Option<String> }
pub struct Graph { /* a .eg file, or a graph in memory */ }
impl Graph {
    pub fn open(path: &Path) -> Result<Graph>;
    pub fn file(&self) -> Option<&Path>;
    pub fn info(&self) -> Info;          // what `everygraph info` prints
    pub fn nodes(&self) -> Table;
    pub fn edges(&self) -> Table;
}
impl GraphRef for Graph { /* … */ }
pub fn save(g: &impl GraphRef, path: &Path, overwrite: bool, run: &Run)
    -> Planned<PathBuf>;
pub fn verify(path: &Path, deep: bool, run: &Run) -> Planned<VerifyReport>;

// ---- everygraph-ingest (#32–#36, #40, #84) --------------------------------

pub enum Input<'a> { File(&'a Path), Table(&'a dyn EdgeTable) }
pub enum SelfLoops { Drop, Keep }
pub enum Duplicates { Events, Collapse, Sum }
pub struct Read<'a> {
    pub source: Option<&'a str>,
    pub target: Option<&'a str>,
    pub weight: Option<&'a str>,
    pub time: Option<&'a str>,
    pub directed: Option<bool>,
    pub self_loops: SelfLoops,           // Drop
    pub duplicates: Duplicates,          // Events
    pub out: Option<&'a Path>,
    pub overwrite: bool,
}
pub fn read(input: Input, read: &Read, run: &Run) -> Planned<Graph>;
pub fn plan_read(input: Input, read: &Read, run: &Run) -> Result<Plan>;
pub fn from_edges(source: &Ids, target: &Ids, directed: bool,
                  self_loops: SelfLoops, duplicates: Duplicates, run: &Run)
    -> Planned<Graph>;

// ---- everygraph-kernels (#44–#51) -----------------------------------------

pub struct PageRank<'a> {
    pub damping: f64,                    // 0.85
    pub tol: f64,                        // 1e-6 †
    pub max_iter: u32,                   // 100 †
    pub weight: Weight<'a>,
    pub personalize: Option<&'a [NodeId]>,
    pub precision: Precision,            // Full
}
pub fn pagerank(g: &impl GraphRef, p: &PageRank, run: &Run) -> Planned<Table>;
pub fn components(g: &impl GraphRef, strong: bool, run: &Run) -> Planned<Table>;
pub fn kcore(g: &impl GraphRef, run: &Run) -> Planned<Table>;
pub fn triangles(g: &impl GraphRef, run: &Run) -> Planned<Table>;
pub fn degrees(g: &impl GraphRef, nodes: Option<&[NodeId]>, weight: Weight,
               run: &Run) -> Planned<Table>;
pub fn bfs(g: &impl GraphRef, source: NodeId, direction: Direction,
           max_hops: Option<u32>, run: &Run) -> Planned<Table>;
pub fn top(g: &impl GraphRef, by: Measure, n: usize, run: &Run)
    -> Planned<Table>;

/// `g.explain(...)` and `dry_run=True`. The planner's crate is #45's choice;
/// it is shown here because the kernels declare their costs to it.
pub enum Operation<'a> {
    PageRank(PageRank<'a>), Components { strong: bool }, KCore, Triangles,
    Bfs { source: NodeId, direction: Direction, max_hops: Option<u32> },
    Communities(Communities<'a>), Map(MapParams<'a>),
}
pub fn explain(g: &impl GraphRef, op: &Operation, run: &Run) -> Result<Plan>;

// ---- everygraph-community (#57, #58, #60, #62, #63) ------------------------

pub enum Method { Leiden }
pub enum Objective { Modularity, Cpm }
pub struct Communities<'a> {
    pub method: Method,
    pub resolution: f64,                 // 1.0
    pub objective: Objective,
    pub iterations: u32,                 // 1 †
    pub weight: Weight<'a>,
    pub use_direction: bool,             // false: direction is ignored
    pub deterministic: bool,             // true
    pub seed: u64,                       // 0
}
pub fn communities(g: &impl GraphRef, p: &Communities, run: &Run)
    -> Planned<Partition>;

pub struct Partition { /* the Communities object of § 3.5 */ }
impl Partition {
    pub fn len(&self) -> u64;
    pub fn modularity(&self) -> f64;
    pub fn levels(&self) -> u32;
    pub fn table(&self, level: u32) -> Table;
    pub fn nodes(&self, level: u32) -> Table;
    pub fn of(&self, nodes: &[NodeId]) -> Table;
    pub fn members(&self, community: u32, level: u32) -> Table;
}
pub struct Hierarchy { /* § 3.6 */ }
impl Hierarchy {
    pub fn levels(&self) -> u32;
    pub fn table(&self, level: Option<u32>) -> Table;
    pub fn of(&self, nodes: &[NodeId]) -> Table;
}

// ---- everygraph-query (#90–#94) --------------------------------------------

pub fn neighbors(g: &impl GraphRef, nodes: &[NodeId], direction: Direction,
                 limit: u64) -> Result<Table>;
pub fn khop(g: &impl GraphRef, node: NodeId, k: u32, direction: Direction,
            limit: u64, timeout: Duration) -> Result<Table>;
pub struct PathParams<'a> {
    pub max_hops: Option<u32>,
    pub avoid: Option<&'a [NodeId]>,
    pub weight: Option<&'a str>,         // a cost column; None: hops
    pub limit: u64,
    pub timeout: Duration,
}
pub fn path(g: &impl GraphRef, source: NodeId, target: NodeId,
            p: &PathParams) -> Result<FoundPath>;
pub struct FoundPath { /* the Path object of § 3.7 */ }
impl FoundPath {
    pub fn found(&self) -> bool;
    pub fn hops(&self) -> Option<u32>;
    pub fn nodes(&self) -> Table;
    pub fn edges(&self) -> Table;
    pub fn alternatives(&self, limit: usize) -> Vec<FoundPath>;
}
pub fn has_edge(g: &impl GraphRef, source: NodeId, target: NodeId)
    -> Result<bool>;                     // reading can fail (I/O)
pub fn common_neighbors(g: &impl GraphRef, a: NodeId, b: NodeId, limit: u64)
    -> Result<Table>;
pub fn edges_between(g: &impl GraphRef, a: &[NodeId], b: &[NodeId],
                     limit: u64) -> Result<Table>;

pub struct View<'g> { /* a definition over a graph; mask or materialized */ }
impl GraphRef for View<'_> { /* … */ }
pub fn filter<'g>(g: &'g impl GraphRef, edges: Option<&str>,
                  nodes: Option<&str>) -> Result<View<'g>>;
pub fn window<'g>(g: &'g impl GraphRef, start: Option<Time>,
                  end: Option<Time>) -> Result<View<'g>>;
pub fn subgraph<'g>(g: &'g impl GraphRef, nodes: &[NodeId])
    -> Result<View<'g>>;
impl<'g> View<'g> {
    pub fn and(&self, other: &View<'g>) -> Result<View<'g>>;
    pub fn definition(&self) -> &Definition;
}

// ---- everygraph-map (#63, #67–#69, #75) ------------------------------------

pub enum Reveal { Full, Summary }        // required: no Default
pub struct MapParams<'a> {
    pub communities: Option<&'a Partition>,
    pub deterministic: bool,
    pub seed: u64,
}
pub fn map(g: &impl GraphRef, p: &MapParams, run: &Run) -> Planned<MapFile>;
pub struct MapFile { /* a .egmap */ }
impl MapFile {
    pub fn file(&self) -> &Path;
    pub fn levels(&self) -> u32;
    pub fn hierarchy(&self) -> &Hierarchy;
    pub fn export(&self, path: &Path, reveal: Reveal, overwrite: bool,
                  run: &Run) -> Planned<PathBuf>;
}
```

How the two APIs differ, on purpose:

| Python | Rust | Why |
|---|---|---|
| Identifiers in and out | `NodeId` in and out; `resolve` and `id_of` convert in bulk | The embedding developer usually has dense numbers already; the conversion is one explicit call instead of one per call. `NodeId` is the engine's internal number, which the Python API never shows (§ 2.2). |
| Exceptions | `Result<T, Error>` | The same kinds and fields. The ones that name a file — `OutputExists`, `CorruptFile` and operating-system errors (`IoError`) — are defined by `everygraph-io` and `everygraph-format` and travel boxed in `Error::Storage`, so that `everygraph-core` has no file-system types (D2). |
| A clean stop | `Outcome::Stopped`, with the reason given to `Cancel` | A stop is not an error: a planned function returns `Planned<T>`, that is `Result<Outcome<T>>`. Python raises after the clean stop: a plain `KeyboardInterrupt` for `Interrupt`, and for `Exception` the very exception the progress callback raised (`docs/ux-contract.md` G-18, C-6). The command exits with 130 or 143 and, with `--json`, prints `Stopped::to_json()`, the stop document. |
| `temp_dir=` | `Run::scratch` | The core asks for scratch through a trait, a handle to one job's directory; the scratch root, the job's directory in it and resuming are the business of `everygraph-io` (`scratch_root`, `DirScratch::for_job`). |
| `progress=` | `Run::events` | The same events. The command and the Python package render them with the one renderer of `docs/ux-contract.md` § 8. |
| `dry_run=True` | `explain`, `plan_read` | A function that returns a `Plan` is clearer in Rust than a flag that changes the return type. |
| `g.top("degree", n)` | `top(g, Measure::Degree, n, run)` | A typed key for the measures; `Table::top` takes a column name, as the table's columns are named. |
| `num_nodes` on a view | `num_nodes(run)` returning `Result` | Counting a view is a planned pass that can fail or be cancelled (§ 3.3). |
| `Selection`, `highlight`, the widget | — | They belong to the viewer (#71, #76), not to the engine. |
| `to_networkx` and the other converters | — | Python-side by nature. |

Constraints the signatures respect (`docs/FINDINGS.md` § G): nothing in
`everygraph-core` names a file, a path or an operating-system thread — `Graph::open`,
`read`, `DirScratch` and the errors that carry a path are in the crates that may touch
the file system, while the kernels see only `GraphRef`, `Run` and `Scratch`; every long
function takes a `Cancel`, returns `Outcome::Stopped` when it is cancelled, and can be
run again; every function that reads the graph
returns a `Result`, because reading can fail or be cancelled; no function returns a
structure that needs a single allocation proportional to the graph.

---

## 5. The command line

The command is `everygraph`; no short alias is installed (`cli.md` Choice 1). Its shape
is verb, then thing (`cli.md` Choice 5). Everything it prints, and where, is specified
in `docs/ux-contract.md`; this section fixes what can be typed and what a result looks
like.

### 5.1 Commands

```text
everygraph ingest FILE   [--source NAME] [--target NAME] [--weight NAME]
                         [--time NAME] [--directed | --undirected]
                         [--self-loops drop|keep]
                         [--duplicates events|collapse|sum]
                         [--ids auto|string] [--out FILE]
everygraph info GRAPH
everygraph run NAME GRAPH    [options of NAME] [VIEW] [--top N] [--out FILE]
everygraph query NAME GRAPH NODE [NODE]
                             [options of NAME] [VIEW] [--top N] [--out FILE]
everygraph map GRAPH     [VIEW] [--out FILE --reveal full|summary]
everygraph open GRAPH
everygraph export GRAPH  [VIEW] [--topology-only] --out FILE
everygraph verify FILE   [--deep] [--strict]
everygraph compact FILE  [--out NEW]
everygraph doctor

VIEW := [--window START END] [--edges EXPRESSION] [--nodes EXPRESSION]
```

| Command | Does | Python | Implemented by |
|---|---|---|---|
| `ingest FILE` | Reads a file of edges and builds `NAME.eg` beside it, `NAME` being the input's name with every recognized extension removed (`transfers.csv.gz` → `transfers.eg`). Always writes a file. An existing one is opened, continued or refused as § 2.7 says. | `eg.read(file, …)` | #98; #32–#36, #40 |
| `info GRAPH` | What the file holds and what was computed so far. | `g.info()` | #98; #39 |
| `run NAME GRAPH` | A whole-graph algorithm. `NAME`: `pagerank`, `components`, `kcore`, `triangles`, `degrees`, `bfs` (takes one `NODE`), `communities`. Its options are the method's parameters (§ 3.2) with dashes. | `g.NAME(…)` | #98; #47–#51, #57 |
| `query NAME GRAPH NODE …` | A question about particular nodes. `NAME`: `neighbors`, `khop`, `path` (takes two nodes). | `g.NAME(…)` | #98; #90, #91 |
| `map GRAPH` | Builds whatever is missing and writes the working map `GRAPH.egmap`, full, beside the graph. With `--out FILE`, writes a map for sharing there instead, with `viewer.html` beside it; `--reveal` is then required. A map of a view needs `--out`, since only the whole graph's map is kept beside it (§ 2.8). | `g.map()`, `m.export(…)` | #98; #63, #67, #69, #75 |
| `open GRAPH` | Serves the map to a browser on this computer only (`127.0.0.1`) until `Ctrl-C`. | displaying `m` | #74 |
| `export GRAPH --out FILE` | Writes the graph, or a view of it, in the format of `FILE`'s extension. With `--topology-only`, only the structure: the edges, without any property column, so that a user can share a failing case without its properties (the feedback loop, #106). The output is **not anonymized**, and the command prints a one-line notice that says so: structure alone can identify people (`docs/research/README.md` H7). | `g.save(path)` | #85; #106 |
| `compact FILE` | Writes a copy of the graph file that holds only what its newest commit uses, and so reclaims the bytes of replaced or removed results (`docs/spec/eg-container.md` § 9; ADR 0027). Without `--out`, the result replaces `FILE` in place: it is written as `FILE.partial` and renamed over `FILE` once durable, so a stop leaves `FILE` as it was. With `--out NEW`, `FILE` is not changed. `info` says how many bytes `compact` would reclaim. | — | #98; #28 |
| `verify FILE` | Checks that a `.eg` or `.egmap` is intact: quickly, or with `--deep` every byte and every invariant. Exit status 5 if not. | — | #41 |
| `doctor` | What this machine offers — cores, memory, storage, limits, graphics — and a memory budget that suits it. | — | #98; #45 |

### 5.2 Options

Options that mean the same on every command that takes them (§ 2.4 has their meaning):

| Option | Python | On |
|---|---|---|
| `--memory SIZE` | `memory=` | every command that computes |
| `--threads N` | `threads=` | every command that computes |
| `--temp-dir DIR` | `temp_dir=` | `ingest`, `run`, `map`, `export` |
| `--dry-run` | `dry_run=True`, `g.explain()` | `ingest`, `run`, `map`, `export` |
| `--json` | `.to_dict()`, Arrow | every command |
| `--out FILE` | `t.write()`, `g.save()`, `m.export()`, `out=` | `ingest`, `run`, `query`, `map`, `export` |
| `--overwrite` | `overwrite=True` | every command that takes `--out`, and `ingest` |
| `--top N` | `.top(n)` | `run`, `query` |
| `--deterministic` / `--fast` | `deterministic=` | `run communities`, `map` |
| `--seed N` | `seed=` | `run communities`, `map` |
| `--weight NAME` / `--no-weight` | `weight=` | `run`; on `ingest`, `--weight NAME` names the weight column |
| `--limit N`, `--timeout SECONDS` | `limit=`, `timeout=` | `query` |
| `--window START END` | `g.window(start, end)` | `run`, `query`, `map`, `export` |
| `--edges EXPRESSION`, `--nodes EXPRESSION` | `g.filter(edges=…, nodes=…)` | `run`, `query`, `map`, `export` |
| `-h`, `--help`; `--version` | — | everywhere |

Options particular to one name:

| Command | Options | Python |
|---|---|---|
| `ingest` | `--source NAME`, `--target NAME`, `--time NAME`, `--directed`, `--undirected`, `--self-loops drop\|keep`, `--duplicates events\|collapse\|sum`, `--ids auto\|string` | the parameters of `eg.read` |
| `run pagerank` | `--damping X`, `--tol X`, `--max-iter N`, `--personalize FILE`, `--precision full\|low` | `g.pagerank(…)` |
| `run components` | `--strong` | `strong=True` |
| `run bfs` | `--direction out\|in\|both`, `--max-hops N` | `g.bfs(…)` |
| `run communities` | `--method leiden`, `--resolution X`, `--objective modularity\|cpm`, `--iterations N`, `--use-direction` | `g.communities(…)` |
| `query neighbors` | `--direction out\|in\|both` | `g.neighbors(…)` |
| `query khop` | `--k N`, `--direction out\|in\|both` | `g.khop(…)` |
| `query path` | `--max-hops N`, `--avoid-hubs N`, `--weight NAME` | `g.path(…)`; `--avoid-hubs N` is `avoid=g.top("degree", N)` |
| `map` | `--reveal full\|summary`, required with `--out` | `m.export(…, reveal=…)` |
| `export` | `--topology-only` | — |
| `compact` | `--out NEW` | — |
| `verify` | `--deep`, `--strict` | — |

`--directed` and `--undirected` on `ingest` declare what the input is; `--use-direction`
on `run communities` asks the algorithm to use the direction the graph has. They are
different words because they mean different things (§ 9). A `FILE` given to
`--personalize` is a text file with one identifier per line, or any table file with a
column named `node`.

`map --out` without `--reveal` is a usage error, and its message gives the two ways
forward:

```console
$ everygraph map transfers.eg --out shared.egmap
✗ A map written for sharing needs --reveal.

  It holds either every node identifier and the links shown, or only
  regions and counts; nothing is chosen for you.

  Ways forward:
    --reveal summary        regions and counts, no identifiers
    --reveal full           every identifier and the links shown

Nothing was written.
$ echo $?
2
```

Python only in 0.1, because they need a session: `eg.from_edges`, `g.subgraph` and the
node sets it takes (`c[i]`, a selection, a `khop` result), `avoid=` with any node set
other than the top hubs, `c.of`, `m.highlight`, `m.selection`, `p.alternatives`, the
converters to NetworkX, igraph and SciPy, and `progress=`.

Not provided, by decision: a confirmation prompt and `--yes` (nothing ever asks); a
quiet or verbose switch (`cli.md` Choice 2); a short alias.

### 5.3 What a result looks like

One result, three forms (`cli.md` Choice 6), chosen as in `docs/ux-contract.md` § 2.2:

- **On a terminal**: a table aligned with spaces, numbers with separators, identifiers
  shortened only if the table would pass 80 columns, and a dim footer (story 2). Without
  `--top`, the first ten rows and `10 of 212,418,907 rows shown`.
- **Piped**: every row, tab-separated, with a header row; plain numbers; full
  identifiers; no footer.
- **`--json`**: standard output carries exactly one JSON document in every outcome
  (`docs/ux-contract.md` S-2) — for `run` and `query`, the result document below, with
  or without `--out`; for `ingest`, `map` and `export`, which make a file, the file
  document of `docs/ux-contract.md` § 4.7; the plan with `--dry-run`; the error
  document of its § 6.1 when the command fails or is refused, a usage error included;
  the stop document of its § 5.4 when it is stopped. Standard error keeps the plan,
  the progress and the warnings; it does not repeat the final message.

The result document holds a summary and the first rows, never the whole table: the
full data leaves through `--out`. Its schema is closed, like those of
`docs/ux-contract.md` § 3.6, and follows the same compatibility rules; the open places
are `summary` and `meta`, whose keys depend on the operation, and the rows.

```json
{
  "schema": 1,
  "result": "pagerank",
  "graph": "transfers.eg",
  "view": null,
  "summary": null,
  "columns": ["node", "pagerank", "rank"],
  "rows": [
    ["0x28c6c06298d514db089934071355e5743bf21d60", 0.00412907, 1],
    ["0xdac17f958d2ee523a2206206994597c13d831ec7", 0.00388114, 2],
    ["0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", 0.00301775, 3],
    ["0x21a31ee1afc51d94c2efccaa2092ad1028285549", 0.00214466, 4],
    ["0x7a250d5630b4cf539739df2c5dacb4c659f2488d", 0.00198203, 5]
  ],
  "rows_returned": 5,
  "rows_total": 212418907,
  "stored_in": "transfers.eg",
  "cached": true,
  "truncated": null,
  "meta": {
    "operation": "pagerank",
    "parameters": {"damping": 0.85, "tol": 1e-06, "max_iter": 100,
                   "weight": true, "precision": "full"},
    "deterministic": true,
    "weight": "value",
    "iterations": 38,
    "converged": true,
    "cached": true
  }
}
```

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:result:1",
  "title": "Everygraph result, version 1",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema", "result", "graph", "view", "summary", "columns", "rows",
    "rows_returned", "rows_total", "stored_in", "cached", "truncated", "meta"
  ],
  "properties": {
    "schema": {"const": 1},
    "result": {"type": "string"},
    "graph": {"type": "string"},
    "view": {
      "oneOf": [
        {"type": "null"},
        {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "window": {
              "type": "array",
              "items": {"type": ["string", "integer", "null"]},
              "minItems": 2,
              "maxItems": 2
            },
            "edges": {"type": "string"},
            "nodes": {"type": "string"}
          }
        }
      ]
    },
    "summary": {"type": ["object", "null"]},
    "columns": {"type": "array", "items": {"type": "string"}},
    "rows": {"type": "array", "items": {"type": "array"}},
    "rows_returned": {"type": "integer", "minimum": 0},
    "rows_total": {"type": "integer", "minimum": 0},
    "stored_in": {"type": ["string", "null"]},
    "cached": {"type": "boolean"},
    "truncated": {
      "oneOf": [
        {"type": "null"},
        {
          "type": "object",
          "additionalProperties": false,
          "required": ["by", "limit", "rows", "where"],
          "properties": {
            "by": {"enum": ["limit", "timeout"]},
            "limit": {"type": "integer", "minimum": 1},
            "timeout_seconds": {"type": "number", "minimum": 0},
            "rows": {"type": "integer", "minimum": 0},
            "where": {"type": ["string", "null"]},
            "total": {"type": ["integer", "null"], "minimum": 0}
          }
        }
      ]
    },
    "meta": {"type": "object"}
  }
}
```

| Key | Holds |
|---|---|
| `schema` | `1`. The compatibility rules of `docs/ux-contract.md` § 3.6 apply: an optional key may be added within a version; removing or renaming one, changing its type or meaning, or adding a value to an enumeration increments it. |
| `result`, `graph`, `view` | The operation, the graph file, and the view's definition (`window`, `edges`, `nodes`) or `null`. |
| `summary` | What the text form prints above the table, as data, or `null` when there is none: for `communities`, `communities`, `modularity`, `disconnected`, `levels` (the sizes from the nodes to the partition), `largest`, `median`, `singletons`, `weight`, `direction`; for `path`, `found`, `hops`, `avoided`, `alternatives`. |
| `columns`, `rows` | The first `--top N` rows (10 when `--top` is not given) in the result's order (§ 2.12). For `run communities` these are the largest communities (`c.table()`); the membership table leaves through `--out`. |
| `rows_returned`, `rows_total` | How many rows are in `rows`, and how many the whole result has. |
| `stored_in` | Where the whole result is: the graph file for a result on the whole graph, the `--out` file when one was given, or `null` when it is kept nowhere (a point query, a result on a view without `--out`). |
| `cached`, `truncated` | As in § 2.8 and § 2.5: `truncated` is `null`, or `by`, `limit`, `rows`, `where` and, when known without further work, `total` (and `timeout_seconds` for a cut by time). |
| `meta` | The table's `meta` (§ 3.4), every key of it; the example above shows a few. |

`info`, `doctor` and `verify` follow the same rule: with `--json`, one document on
standard output with a `schema` key. The schemas of `info` and `doctor` are #98's to
write (§ 7). That of `verify` (#41) holds the report whether the file is intact or not
(exit status 0 or 5); a file that cannot be read gives the error document instead:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "urn:everygraph:schema:verify:1",
  "title": "Everygraph verify report, version 1",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema", "verify", "deep", "intact", "generation", "lost_generation",
    "sections", "sections_hashed", "bytes_hashed", "problems", "notes", "unchecked"
  ],
  "properties": {
    "schema": {"const": 1},
    "verify": {"type": "string"},
    "deep": {"type": "boolean"},
    "intact": {"type": "boolean"},
    "generation": {"type": ["integer", "null"], "minimum": 1},
    "lost_generation": {"type": ["integer", "null"], "minimum": 2},
    "sections": {"type": "integer", "minimum": 0},
    "sections_hashed": {"type": "integer", "minimum": 0},
    "bytes_hashed": {"type": "integer", "minimum": 0},
    "problems": {"type": "array", "items": {"$ref": "#/$defs/finding"}},
    "notes": {"type": "array", "items": {"$ref": "#/$defs/finding"}},
    "unchecked": {"type": "array", "items": {"type": "string"}}
  },
  "$defs": {
    "finding": {
      "type": "object",
      "additionalProperties": false,
      "required": ["section", "problem"],
      "properties": {
        "section": {"type": ["string", "null"]},
        "problem": {"type": "string"}
      }
    }
  }
}
```

`verify` is the file checked, as given; `generation` is the commit it reads as, `null`
when it is not a graph file; `lost_generation` is a newer commit the file holds but
cannot read (its table or its header block is damaged, or its commit was cut short),
`null` otherwise; `intact` is true exactly when `problems` is empty. A lost generation
is a warning, status 0, or status 5 with `--strict` (`design/DECISIONS.md`, "A lost
newest commit").
`section` is `null` for the header and the table of contents. `notes` are what may be
damage or the trace of a commit cut short, which leaves the file whole; `unchecked`
says which checks could not be made (a declaration the file's provenance lacks).

**Which stream** (`docs/ux-contract.md` § 2.1). A result's summary is the command's
result and goes to standard output: `run communities` prints the summary of § 3.5 there,
on a terminal, piped (in the piped rendering) and with `--out`; with `--top N` the
table of the `N` largest communities follows it; with `--json` the document's `summary`
holds it. The membership table (`node`, `community`) leaves through `--out`. `query
path` prints the two or three lines of story 3 on standard output. `info` and `doctor`
print labels and values on standard output. The summary of an input that `ingest`
prints is on standard error, with the plan and the progress. With `--out FILE` the
table goes to the file and the result line, on standard error, says what was written. A
dry run's plan is its result: all of it is on standard output, the closing `Dry run:
nothing was written.` and its hint included.

`everygraph --help`, as typeset in `terminal.html`, with the lines this specification
adds (`export`, `verify`, `--threads`, `--temp-dir`, `--overwrite`, and the longer lists
after `run` and `query`):

```console
$ everygraph --help
Everygraph 0.1.0 — ingest, analyze, query and map a graph on one machine.

Usage
  everygraph COMMAND [OPTIONS]

Commands
  ingest FILE           read a file of edges and build a graph (.eg) beside it
  map GRAPH             find communities, lay them out, write a map (.egmap)
  open GRAPH            show the map in your browser, on this computer only
  run NAME GRAPH        analyze the whole graph: pagerank, communities, …
  query NAME GRAPH …    ask about particular nodes: neighbors, khop, path
  info GRAPH            what a graph file holds and what was computed so far
  export GRAPH          write the graph, or part of it, in another format
  verify FILE           check that a graph or map file is intact
  doctor                what this machine offers, and a budget that suits it

Options
  --memory SIZE         the memory budget, for example 8GB
  --threads N           the most threads to use
  --temp-dir DIR        where scratch files go
  --dry-run             print the plan and the estimate, then stop
  --json                print the result as JSON, with nothing around it
  --out FILE            write the result to a file, for example pr.parquet
  --overwrite           replace a file that is not one Everygraph can recognize
  -h, --help            this text; after a command, the help for that command
  --version             print the version

Start here
  everygraph ingest FILE
```

### 5.4 Exit status

The statuses of `docs/ux-contract.md` § 6.4, repeated here word for word. In Python
the same cases are the exceptions of § 2.7, and an operating-system error stays
Python's own `OSError` subclass; `IoError` and `UsageError` are classes of the command
line only, so that `--json` can describe those failures too:

| Status | Meaning |
|---|---|
| 0 | Done. Includes a result cut short, and a dry run whose plan fits. |
| 1 | Failed for a reason not listed below: an operating-system error reading or writing a file (`IoError`), an output file that may not be replaced (`OutputExists`), another run of the same job in progress or an internal error (`EverygraphError` with no more specific class). |
| 2 | Usage: the command line could not be understood or cannot be honoured — an unknown command or option, a missing argument such as `--reveal` with `map --out` (`UsageError`), a budget larger than the machine's memory, a filter that does not parse (`InvalidExpression`), an operation the graph does not support (`Unsupported`). |
| 3 | The input is not valid (`InvalidInput`): rows that cannot be read, or a weight column with negative, NaN or infinite values where an operation needs weights. |
| 4 | Refused before starting for lack of memory or disk (`InsufficientBudget`, `InsufficientDisk`), or the disk fills during a run (`InsufficientDisk`). Also a dry run whose plan does not fit. |
| 5 | A graph or map file is damaged (`CorruptFile`), or `everygraph verify` found a problem. |
| 6 | No such node: an identifier that is not in the graph or the view (`UnknownNode`; `design/DECISIONS.md`, "Exit status for 'no such node'"). |
| 130 | Stopped by `Ctrl-C` (SIGINT). |
| 143 | Stopped by SIGTERM. |

---

## 6. Every public name, its issue and its story

"Story" is the story of § 1 that exercises the name; a dash means no story does, and the
name is covered by the tests of its own issue only. ‡ and † as in § 3. Names are written
qualified (`g.top()`, `Table.top()`), so that each row names one thing.

**Python: functions and `Graph`**

| Name | Implemented by | Story |
|---|---|---|
| `eg.read` | #81; #32, #33, #34, #35, #36, #40 | 1, 2, 9 |
| `eg.read` with a table as `data` | #83, #95 | — |
| `eg.open` | #81; #28, #41 | 3 |
| `eg.from_edges` ‡ | #84 | 10 |
| `eg.from_scipy` ‡ | #84 | — |
| `eg.__version__` | #81 | — |
| `Graph`: `g.num_nodes`, `g.num_edges`, `g.directed`, `g.file` | #81, #93 | 4 |
| `g.columns` | #37 | — |
| `g.info()` | #39, #98 | — (§ 3.2) |
| `g.nodes()`, `g.edges()` | #81, #83 | — |
| `g.save()` ‡ | #84, #85 | 7, 10 |
| `g.pagerank()` | #47 | 2, 5, 6 |
| `g.components()` | #48 | — |
| `g.kcore()` | #50 | — |
| `g.triangles()` | #51 | 10 |
| `g.degrees()`; the columns `in_degree`, `out_degree`, `degree`, `weighted` | #50, #90 | 5 |
| `g.bfs()` | #49 | — |
| `g.top()` | #81; #50, #47 | 3 |
| `g.communities()` | #57, #58, #60, #62 | 1, 4, 6, 7, 8 |
| `g.map()` | #63, #67, #69 | 5 |
| `g.explain()` | #45 | 8 |
| `g.neighbors()` | #90, #92 | 3 |
| `g.khop()` | #90, #92 | 3, 7 |
| `g.path()` | #91, #38, #92 | 3 |
| `g.has_edge()`, `g.common_neighbors()`, `g.edges_between()` ‡ | #90 | — |
| `g.filter()`, its `edges=` and `nodes=` | #93, #37 | 4 |
| `g.window()` | #94, #93 | 4 |
| `g.subgraph()` | #87, #93, #62 | 7 |
| `g.to_networkx()` ‡ | #85, #87 | 7 |
| `g.to_igraph()`, `g.to_scipy()` ‡ | #85 | — |

**Python: the other objects**

| Name | Implemented by | Story |
|---|---|---|
| `View`, `view & view`, `view.base`, `view.definition` | #93 | 4 |
| `Table.columns`, `Table.num_rows`, `len(t)` | #81 | — |
| `Table.truncated` | #90 | 3 |
| `Table.meta` | #45, #39 | — |
| `Table.head()` | #81 | 1 |
| `Table.top()`, its `by=` | #47, #81 | 2, 5, 6 |
| `Table.sizes()` | #48, #50 | — |
| `Table.on_map()` | #76 | — |
| `Table.write()` | #83, #85 | 1 |
| `Table.to_arrow()`, `Table.to_pandas()` | #83 | — |
| the Arrow interface of `Table` and `Communities` | #83 | 6 |
| `len(c)`, `c.modularity`, `c.levels`, `c.meta` | #57, #62 | 1 |
| `c.table()` | #62 | 1 |
| `c.nodes()` | #57, #62 | 1, 6 |
| `c.of()` | #62 | — |
| `c[i]`, `Community.id`, `Community.nodes()`, `len(Community)` | #62, #87 | 7 |
| `Hierarchy.levels`, `Hierarchy.table()` | #62, #63 | — |
| `Hierarchy.of()` | #62 | — |
| `Path`: `p.found`, `p.hops`, `p.truncated`, `p.meta` | #91 | 3 |
| `p.nodes()`, `p.edges()` | #91 | — |
| `p.alternatives()` | #91 | 3 |
| `p.on_map()` | #76 | — |
| `Map`, displaying it | #76, #71, #78 | 5 |
| `m.export()` | #75 | 5 |
| `m.highlight()` | #76 | 5 |
| `m.selection`, `Selection.kind`, `Selection.nodes()` | #76 | 5 |
| `m.hierarchy`, `m.file`, `m.levels` | #63, #69 | — |
| `Plan.fits`, `Plan.operation`, `Plan.estimate`, `Plan.refusal`, `Plan.to_dict()` | #45, #40 | 8 |
| `EverygraphError.message`, `EverygraphError.why`, `EverygraphError.arithmetic`, `EverygraphError.ways_forward`, `EverygraphError.changed`, `EverygraphError.kept`, `EverygraphError.machine_memory_bytes`, `EverygraphError.to_dict()` | #81 | 8 |
| `Way`, with `command`, `python`, `note`, `needs_bytes` | #81 | 8 |
| `InsufficientBudget` | #31, #45 | 8 |
| `InsufficientDisk` | #40 | — |
| `InvalidInput` | #32, #33 | — |
| `InvalidExpression` | #93 | — |
| `UnknownNode` | #34, #90 | — |
| `Unsupported` | #81; #85, #94 | 7 |
| `OutputExists` | #81; #28, #39 | — |
| `CorruptFile` | #28, #41 | — |
| `TruncatedWarning` | #81, #90 | 3 |

**Parameters and environment**

| Name | Implemented by | Story |
|---|---|---|
| `memory=`, `--memory`, `EVERYGRAPH_MEMORY` | #31, #45 | 8 |
| `threads=`, `--threads`, `EVERYGRAPH_THREADS` | #45 | — |
| `temp_dir=`, `--temp-dir`, `EVERYGRAPH_TEMP_DIR` | #40 | 9 (the default) |
| `deterministic=`, `--deterministic`, `--fast` | #57, #54 | 1 (default) |
| `seed=`, `--seed` | #57 | — |
| `precision=`, `--precision` of `pagerank` | #45, #47 | — (story 8 names it) |
| `weight=`, `--weight`, `--no-weight` | #36, #47, #58 | 2 (`eg.read`), 4 |
| `limit=`, `--limit` | #90, #91 | 3, 7 |
| `timeout=`, `--timeout` | #90, #91 | — |
| `dry_run=`, `--dry-run` | #40, #45 | 8 |
| `progress=` | #81 | — |
| `overwrite=`, `--overwrite` | #81, #98; #28 | — |
| `direction=`, `max_hops=`, `avoid=`, `k=` | #90, #91 | 3 |
| `source=`, `target=`, `time=`, `directed=` of `eg.read`; `--source`, `--target`, `--time`, `--directed`, `--undirected` | #32, #33, #36 | 2 |
| `self_loops=`, `duplicates=`; `--self-loops`, `--duplicates` | #36 | 1 (the defaults) |
| `ids=`; `--ids` | #34, #98 | — |
| `damping=`, `tol=`, `max_iter=`, `personalize=`; `--personalize` | #47 | — |
| `strong=`, `--strong` | #48 | — |
| `method=`, `resolution=`, `objective=`, `iterations=`, `use_direction=` of `communities`; `--use-direction` | #57, #58 | 4 (`use_direction`, default) |
| `communities=` of `g.map()` | #63 | — |
| `max_edges=` | #85, #87 | 7 |
| `reveal=`, `--reveal` | #75 | 5 |
| `out=`, `--out` | #81, #98 | 1, 6, 7 |

**Command line**

| Name | Implemented by | Story |
|---|---|---|
| `everygraph ingest` | #98; #32–#36, #40 | 1, 2, 9 |
| `everygraph info` | #98; #39 | — (§ 3.2) |
| `everygraph run` with `pagerank`, `communities` | #98; #47, #57 | 1, 2, 4, 6, 8 |
| `everygraph run` with `components`, `kcore`, `triangles`, `degrees`, `bfs` | #98; #48–#51 | — |
| `everygraph query` with `neighbors`, `khop`, `path` | #98; #90, #91 | 3 |
| `everygraph map` | #98; #63, #67, #69, #75 | 5 |
| `everygraph open` | #74 | 5 |
| `everygraph export` | #98; #85 | 7 |
| `everygraph verify`, `--deep` | #41 | — |
| `everygraph doctor` | #98; #45 | — |
| `--json` and the result document | #98 | — |
| `--top` | #98 | 2 |
| `--window`, `--edges`, `--nodes` | #98; #93, #94 | 4, 7 |
| `--avoid-hubs` | #98; #91 | 3 |
| exit statuses, 6 included | #98 | 8 |

The Rust names of § 4 map crate by crate: `everygraph-core` #2, #31, #44, #45;
`everygraph-io` #29; `everygraph-format` #28, #39, #41; `everygraph-ingest` #32–#36,
#40, #84; `everygraph-kernels` #44–#51; `everygraph-community` #57, #58, #60, #62, #63;
`everygraph-query` #90–#94; `everygraph-map` #63, #67–#69, #75. The stories exercise
them through Python (#81) and the command (#98).

Every story maps to issues in M2–M7, as the acceptance of #12 requires:

| Story | M2 | M3 | M4 | M5 | M6 | M7 |
|---|---|---|---|---|---|---|
| 1 A file to communities | #32, #34, #35, #36 | | #57, #58, #62 | | #81, #83, #84 | |
| 2 Larger than memory | #33, #35, #39, #40 | #45, #46, #47, #53 | | | #81 | |
| 3 Neighbors and a path | #38 | #50 | | | #81 | #90, #91, #92 |
| 4 Between two dates | #37 | #45 | #57 | | #81 | #93, #94 |
| 5 Map and share | | | #57, #60, #63 | #67, #68, #69, #75, #78 | #76 | #74 |
| 6 Joined with my table | #39 | #47 | #57 | | #83 | #95 |
| 7 A piece to NetworkX | | | #62 | | #85, #87 | #90, #93 |
| 8 Does not fit | #31 | #45, #53 | #60 | | #81 | |
| 9 Interrupted, resumed | #28, #40 | | | | #81 | |
| 10 From arrays | | #51 | | | #83, #84 | |

(#76 and #74 are listed under the milestone they are scheduled in, M6 and M7.)

---

## 7. Gaps

Where the previews, the issues and the design do not line up. Nothing below was invented
or dropped silently: each line says what this document does about it.

**Shown in a preview, with no issue that clearly builds it**

| What | Where | Here |
|---|---|---|
| A low-memory variant of community detection (`precision="low"`: "needs 3.4 GB; slightly lower quality"). No issue builds one; #57 needs 64-bit weights. | `api.md` § 6, `cli.md` § 2, `notebook.html` cell 7, `terminal.html` | Not offered (`design/DECISIONS.md`, "A low-memory variant of community detection"): `communities()` has no `precision`, and its refusal offers only a larger budget (story 8). `precision="low"` stays on `pagerank`, which #47 builds. |
| A unit on path weights (`12.400 ETH`). Nothing in the API declares a unit. | `api.md` § 4 | The weight is labelled with its column's name (§ 9, item 4). |
| Automatic names for regions ("around 0x28c6…1d60 and 2 others", or from a `label` column). #62 computes per-community summaries "for map labels"; no issue owns the naming rule. | `choices.md` B3 | `Hierarchy.table()` has a `name` column; who fills it is open. Suggested: #63. |
| The user's own nouns in summaries ("addresses", "transfers"). No rule says how they are derived. | `design/preview/README.md` N3, `SPEC.md` "Words" | The noun for nodes comes from the kind of identifier (`hex addresses` → "addresses"); no noun is derived for edges in text. A fuller rule belongs to #121. |
| Node columns. `g.filter(nodes=…)`, colouring a map by a label and B3 all assume nodes can have columns, but every preview reads edges only. | `api.md` § 5 | `nodes=` works on `in_degree`, `out_degree`, `degree` and on node columns if a format supplies them (GraphML, GEXF). A way to attach a node table (`eg.read(…, nodes=table)`) needs a preview. |
| `viewer.html` beside a shared map; #75 wrote "a single `index.html` embedding the viewer". | `cli.md` § 1 | `viewer.html`, as approved. #75's text now says the same (amendments, below). |

**Promised by an issue, not exposed here (or exposed differently)**

| What | Issue | Here |
|---|---|---|
| `Graph.view(filter)`; `khop(…, budget=…)`; "requests carry budgets"; "budget 10k nodes" | #81, #90, #74, #87 | The approved names are `g.filter()`, `limit=`, `memory=`. The issue texts predated the decision that "budget" means memory only; #81, #74 and #87 now use the approved names, and #90 still says "stops at the budget" for `limit`. |
| "quiet structured logs" when not on a terminal | #98 | The approved piped form is a plain log; `--json` gives one document (`docs/ux-contract.md` § 9). #98 now says "no `--yes`", since nothing asks. |
| `--json` "validated against a schema" for every command | #98 | The result document has its schema here (§ 5.3, `urn:everygraph:schema:result:1`); the plan, event, error, stop and file documents are in `docs/ux-contract.md`. The schemas of `info` and `doctor` are #98's to write, with a `schema` key like the others; that of `verify` is in § 5.3 (#41). |
| "User overrides for every choice" of the planner | #45 | Exposed: `memory`, `threads`, `temp_dir`, `precision` (PageRank), `deterministic`. Forcing a regime or a decode tier has no public name in 0.1; if #45 needs one for testing it stays out of the documented API. |
| "Pick a lower-memory variant (e.g. f32 state …)" when state does not fit | #45 | Not done by the planner (§ 2.11): it refuses and offers the variant. #45 now says the same (amendment below). |
| Parallel edges merged into one with "count, sum/min/max of a weight, first/last timestamp" | #36 | `duplicates="sum"`, printed `duplicates summed`: the summed weight, the count and the first and last time. Minimum and maximum have no name in 0.1. |
| Weighted single-source distances; multi-source traversal | #49 | `bfs` (hops) only. They remain internal building blocks until a name is previewed. |
| Betweenness, closeness, harmonic and eigenvector centrality | #52 (P2, after 0.1); eigenvector #47 | Not in § 3. By the naming rule they would be `g.betweenness()`, `g.closeness()`, `g.harmonic()`, `g.eigenvector()`. Eigenvector centrality is exact and comes in 0.1 with PageRank (#47). The other three are sampled, come after 0.1 (#52, M9), and mark their results as § 2.8 and the DECISIONS row "Approximate results" say. |
| A quick preview by label propagation | #65 (P2) | Not exposed: `method=` takes `"leiden"` only in 0.1. A second method is added under the same parameter when #65 is built. |
| The giant component and the k-core "as a reusable view" | #48, #50 | Reachable today as `g.subgraph(t)` with a table filtered in Polars. Filters over computed columns (`nodes="component == 0"`) would be the short form; not specified. |
| Degree histogram and quantiles | #50 | The `degrees()` table; statistics are left to the table library. |
| Comparing partitions (NMI, ARI), per-community metrics, stability reports | #64 (P2) | Not in § 3; no previewed names. |
| Activity series and counts per time bucket | #94 | Not in 0.1's API: the series have no previewed names. (`g.as_of(t)` was dropped: `g.window(None, t)` is the same, and #94 now says so.) |
| "Results … back as a node property" | #83 | No call attaches a table to a graph (it is immutable). The join happens in the table library (story 6); node sets come back through `subgraph`, `highlight`, `avoid`. |
| `eg.from_dataframe` | #84 | Folded into `eg.read(table)`; #84 now says the same. |
| Batch forms "returning Arrow list columns" | #90 | A flat table with a leading `node` column: the same shape as every other result, and it joins directly. #90 now says the same. |
| All shortest paths, k shortest paths, a batch of pairs | #91 | `p.alternatives()` covers the first. The other two have no name here. |
| `g.subgraph(…)` returning "a small `Graph` (in memory)" | #87 | It returns a `View`, as #62 and the preview have it; the planner materializes it when small. #87 now says the same. |
| A `MultiGraph` with one edge per event from `to_networkx` | #85 | Not in 0.1: each connected pair becomes one edge carrying the count and the summed weight. |
| `everygraph map export`; `everygraph compact` | #75; #39, #28 | `everygraph map GRAPH --out FILE --reveal …` for the first (#75 now says so). `compact FILE [--out NEW]` is a command of #98 (ADR 0027; § 5.1 and #98 now say so; `design/DECISIONS.md`, "The compact command"), and `info` reports the bytes it would reclaim. It is not yet in the approved help (`design/preview/cli.md`), which #98 updates. |
| NumPy views of numeric columns | #83 | Through Arrow (`pl.from_arrow(t)["pagerank"].to_numpy()` does not copy); no direct `t["pagerank"]` in 0.1. |
| A DuckDB extension with table functions | #95 | Outside the Python API; #95 scopes it as a prototype. |

**Between the design documents**

| What | Here |
|---|---|
| Threads by default: `choices.md` A5 says "all physical cores"; `DESIGN.md` D13 lets compute-bound kernels use every hardware thread. | § 2.4: physical cores is the default and the bound the user sees; the planner may add the sibling threads only when `threads` is not given. |
| Results "stored in transfers.eg" (`cli.md`) against #39, whose text left open the file or a companion directory. | Settled by ADR 0027: results are sections appended to the graph file and committed (`docs/spec/eg-container.md` § 8), as § 2.8 says, and #39 now says the same. Results on views are kept for the session only, as #39 asks ("large temporary results should not bloat the graph file"). |
| `duckdb.sql("select … from pr")` on an Everygraph table (`api.md` Choice 4) relies on DuckDB accepting any Arrow-stream object by variable name. | Kept as approved; #95 must test it, and fall back to documenting `duckdb.from_arrow(pr)` if it does not hold. |
| `docs/DESIGN.md` § 3 names no crate that a Rust user would depend on to get everything. | § 4 writes the functions under the crates that own them. A facade crate `everygraph` would be a small addition to #2. |
| `docs/spec/graph-semantics.md` (#36) defines what each algorithm does on directed, weighted and repeated edges. | § 2.4 (`weight`), § 3.3 (which nodes a view keeps) and § 2.13 state the user-visible behaviour, settled by `design/DECISIONS.md`. The specification follows them; the disagreements it found are in its § 9 and are corrected here. |
| Scratch "in /tmp" in the previews' plans and stop blocks; `design/DECISIONS.md` puts it beside the output. | `61 GB scratch` and `.everygraph-7c1e` beside the output, as `docs/ux-contract.md` § 3.4 and § 5.4 have it. |
| The previews' shared-map hint, `share it: upload transfers.egmap and viewer.html to any static host`, points at the working map, which is full. | The hint is the export command with `--reveal summary` (story 5, § 9 item 21). |

**Amendments other issues need.** This document cannot change issue files; these
sentences are for the coordinator to apply in `plan/issues/` and republish. The first
five hold the kernels to § 2.11. All of them are now in the issue files.

| Issue | Amendment |
|---|---|
| #47 `m3-04-pagerank.md`, Acceptance | Add: "With f64 state the result has identical bytes (the hash of #54) at 1, 4 and 16 threads, in each of the three regimes, at memory budgets of 2, 8 and 16 GiB and unlimited, and on a view whether it is held as a mask or materialized; f32 state is used only when `precision="low"` is asked for, never chosen by the planner." |
| #48 `m3-05-components.md`, Acceptance | Add: "Component labels, numbered from 0 by decreasing size with ties broken by the earliest node in graph order (`docs/api-sketch.md` § 2.12), have identical bytes at 1, 4 and 16 threads, with Afforest in memory and with streaming union-find in the semi-external regime, at memory budgets of 2, 8 and 16 GiB and unlimited, and on a view held as a mask or materialized." |
| #54 `m3-11-determinism-ci.md`, Scope and Acceptance | Replace "document where results are bit-identical vs tolerance-bounded across CPU architectures (x86 vs ARM FMA differences)" with "make results bit-identical across the supported CPU architectures (x86 vs ARM FMA differences); an exception needs an ADR that narrows `docs/api-sketch.md` § 2.11". Add to Acceptance: "The CI job also runs each kernel under two memory budgets that force different regimes (in memory and out of core) and on one view held both as a mask and materialized; every hash equals that of the in-memory run." |
| #93 `m7-04-views.md`, Acceptance | Add: "For every kernel, the result on a view has the same hash (#54) whether the planner holds the view as a mask or materializes it, and the same as on that view saved with `save()` and opened again. A view's `num_nodes` and `num_edges` are computed by one planned pass on first use and never by printing the view." |
| #45 `m3-02-planner.md`, Rules and Acceptance | Replace the first rule with: "State does not fit the budget → use a regime or an algorithm that returns the identical result in less memory (streaming union-find instead of Afforest), or refuse with the minimum budget. A lower-precision variant (f32 state) is never picked by the planner: the refusal offers it as a way forward (`docs/api-sketch.md` § 2.11)." Add to Acceptance: "A plan that changes regime, or holds a view as a mask instead of materializing it, returns a result with the same hash (#54) as the in-memory plan." |
| #34 `m2-07-id-mapping.md`, Scope | Add to the id-assignment bullet: "Graph order — the order of `g.nodes()`, by which results are ordered and ties broken (`docs/api-sketch.md` § 2.2, § 2.12) — is fixed when the file is built and is the same for the same input and parameters at any thread count or budget; a later renumbering may change internal ids, never graph order." Add: "Keys recognized as hexadecimal come back as lowercase text with the prefix the input used; look-ups accept any case." |
| #36 `m2-09-graph-semantics.md`, Scope | Add: "The options are public as `self_loops="drop" \| "keep"` and `duplicates="events" \| "collapse" \| "sum"` (`--self-loops`, `--duplicates`; `docs/api-sketch.md` § 3.1); `sum` keeps one edge per pair with the summed weight, the count and the first and last time. The plan and the summary print them `duplicates kept as events`, `duplicates collapsed`, `duplicates summed`. The build records each numeric edge column's minimum, maximum and count of NaN, so that an operation on the whole graph can refuse a negative, NaN or infinite weight before any work and accept a clean column without reading it (`InvalidInput`, `docs/api-sketch.md` § 2.13)." |
| #50 `m3-07-kcore-degrees.md`, Scope | Replace "Degree columns (in, out, total, weighted)" with "Degree columns `in_degree`, `out_degree`, `degree` (distinct neighbors; `degree` counts each neighbor once whatever the direction) and `weighted`". Add: "On a view they are computed by a planned pass." |
| #90 `m7-01-point-queries.md`, Scope and Acceptance | Replace "`khop(v, k, budget)`" with "`khop(v, k, limit=, timeout=)`; rows are ordered by distance, then by discovery order — the position of the node's parent in the previous distance's order, then the node's position in that parent's neighbor list (graph order) — and the traversal stops as soon as `limit` rows exist, so its cost is bounded by what it returns (`docs/api-sketch.md` § 2.12)". Replace "returning Arrow list columns" with "returning one flat table with a leading `node` column, in input order, `limit` per node". Add to Acceptance: "A `khop` cut at `limit` returns the same rows at 1, 4 and 16 threads, and on hub nodes of twitter-2010 and Friendster its time grows with `limit`, not with the size of the hop it stops in." |
| #91 `m7-02-shortest-paths.md`, Acceptance | Add: "Among several shortest paths the one returned, and the order of the alternatives, follow a documented rule and are the same at any thread count, regime and budget (tested). A step reports the earliest edge of its pair inside the view — for a weighted path the cheapest, earliest among equal costs — and the pair's number of edges." |
| #94 `m7-05-temporal.md`, Scope | Replace "`g.window(t0, t1)` and `g.as_of(t)` as views" with "`g.window(t0, t1)` as a view, closed at both ends, a date meaning its whole day (`docs/api-sketch.md` § 2.3); `as_of` is not public in 0.1 (`g.window(None, t)`)". |
| #39 `m2-12-artifact-cache.md`, Scope | Add: "Results on the whole graph are stored; results on a view are kept for the session only and written nowhere unless the user writes them out (`docs/api-sketch.md` § 2.8)." |
| #98 `m8-01-cli.md`, Scope and Acceptance | Add to Scope: "Exit status 6 for `UnknownNode`; `--overwrite`; `ingest --self-loops`, `--duplicates`; `run communities --use-direction`; `map --out FILE` requires `--reveal`." Add to Acceptance: "The result document is validated against `urn:everygraph:schema:result:1` (`docs/api-sketch.md` § 5.3) and the document of `ingest`, `map` and `export` against `urn:everygraph:schema:file:1` (`docs/ux-contract.md` § 4.7); errors of the operating system are reported as `IoError` with exit status 1; `info`, `doctor` and `verify` have schemas of their own, written by this issue." |
| #75 `m5-09-static-export.md`, Scope | Replace "`everygraph map export` → `.egmap` file + a single `index.html` embedding the viewer" with "`everygraph map GRAPH --out FILE --reveal full\|summary` and `m.export(path, reveal=…)` → `.egmap` file + `viewer.html` beside it; `reveal` is required, and leaving it out is an error that gives the two ways forward." |
| #84 `m6-04-in-memory-constructors.md`, Scope | Add to the spill bullet: "the temporary `.eg` goes to the scratch directory (`temp_dir`, `docs/api-sketch.md` § 2.4) and is removed when the graph object is released, unless `g.save()` keeps it." |

---

## 8. Non-goals for 0.1

Decided, and deliberately absent from everything above.

| Not in 0.1 | Why, and where it went |
|---|---|
| **Changing a graph**: adding or removing nodes or edges, editing a column, appending new data to a file. | A graph is immutable and file-backed (#81, `docs/DESIGN.md` § 2: not a transactional database). A changed input is read again. Appends are designed in #94 and built after 0.1 (#111); the 0.1 format already reserves room for them (ADR 0028, `docs/spec/eg-container.md` § 13). |
| **A query language** (GQL, Cypher, SQL over paths). | `docs/DESIGN.md` D21: traversal calls with limits in 0.1; a GQL subset later (#110). Filters are short comparisons, not a language (§ 3.3). |
| **A NetworkX backend** (`eg.nx`, `nx-everygraph`, `nx.pagerank(G)` running in Everygraph). | Deferred by decision (`choices.md` D5, `DECISIONS.md`; #86 is in M9). The hand-over goes the other way in 0.1 (story 7). |
| Expression objects for filters (`eg.col("value") > 10`). | `api.md` Choice 9: text only. |
| Functions instead of methods (`eg.pagerank(g)`), grouped namespaces (`g.centrality.…`), NetworkX-style long names. | `api.md` Choices 3 and 13. |
| Asking before a long job; a short command alias; translated messages; telemetry of any kind. | `api.md` Choice 2, `cli.md` Choices 1, 3 and 10, `choices.md` D7. |
| A hidden cache directory. | `api.md` Choice 1. |
| A server, remote graphs, more than one machine. | `docs/DESIGN.md` § 2. `everygraph open` listens on this computer only. |
| A browser build, an MCP server, a desktop application, GPU layout, graphs of more than 2³² nodes. | After 0.1: #108, #109, #113, #112, #114. |
| Graph neural networks, a figure editor, styling of small graphs. | `docs/DESIGN.md` § 2: it exports to the tools that do these (story 7, #85). |
| Parity on Windows for out-of-core work. | `choices.md` D6: builds and is tested, without that performance work. |

---

## 9. Decided here

The previews left these open or contradicted each other. Each was chosen with one line
of reasoning. `design/DECISIONS.md` makes this list part of the record (row "Choices
made while writing the two M0 specifications"); the rows of 2026-11-25 that settled the
questions of the independent review are cited where they apply. A departure from an
approved preview says so, with its reason.

**A. Choices**

1. **`explain` is a method and `dry_run=True` is the same thing.**
   `g.explain("communities", …)` returns a `Plan`; so does any planned call with
   `dry_run=True`, and `eg.read(…, dry_run=True)` is the form for reading, where there is
   no graph yet. *`notebook.html` shows the method, `api.md` the flag, #45 and #81 the
   word "explain": one object, two spellings, no third.*
2. **`m.selection.nodes()` returns a table** — `node`, `community`, `in_degree`,
   `out_degree`, `degree` — and an empty table, not an error, when nothing is selected.
   *As drawn in `notebook.html` (cell 11 and "an empty result"), with the degree names
   of item 15; a table is what every other call returns.*
3. **`precision` takes `"full"` (default) or `"low"`, on `pagerank` only.** *"low" is the
   approved word; "full" says the default is not an upgrade. No other operation has the
   variant in 0.1 (DECISIONS, "A low-memory variant of community detection"), so none
   takes the parameter.*
4. **What prints.** Tables have no `shape:` line and always a footer (rows shown of
   total, order); more than ten rows show five, `…` and the last; numbers are
   right-aligned, `share` included. A graph prints the head of `everygraph info`. A path
   labels its weight with the column's name instead of `ETH`, shows the earliest edge of
   each pair and `· N events` when there are several. The summary of communities has a
   `weight` line. *`api.md` showed three table layouts; the typeset cards all have the
   footer. The engine knows a column's name, never its unit. The result must say which
   weight was used (DECISIONS, "Default weight"); the `weight` line departs from
   `api.md` § 3 for that reason.*
5. **Numbering and ties.** Communities and components are numbered from 0 by decreasing
   size, at every level, ties by the earliest node in graph order; levels are numbered
   from 1 at the coarsest, as on the map ("level 1 of 6"), so level 1 of `Communities`
   is the partition; every result has the order of § 2.12. *`c[0]` and "community 0" in
   the approved tables are the largest; a deterministic result must be the same table,
   not only the same set.*
6. **`weight=True` by default**: the weight column declared at `eg.read` if there is
   one, otherwise the number of edges between a pair; the result says which (DECISIONS,
   "Default weight for PageRank and communities"). *`choices.md` A1 says a pair is
   analysed once "with a weight (count or summed amount)"; a column the user declared is
   an instruction.*
7. **Views.** `g.subgraph()` returns a `View`. Filtering edges or taking a window keeps
   only the nodes that still have an edge; filtering nodes keeps the edges between them
   (DECISIONS, "Nodes in a view"). A window is closed at both ends and a date means its
   whole day (DECISIONS, "Time windows"). A view's counts are computed on first use,
   never by printing it; results on a view live for the session. *Communities of one
   month should not report two hundred million isolated nodes; a view that "costs
   nothing to make" must not cost a pass to print; ad-hoc views must not grow the file.*
8. **The map file.** `g.map()` writes the working map `NAME.egmap` beside the graph,
   full, exactly as `everygraph map` does; `m.export(path, reveal=…)` writes the copy to
   share, and `reveal` has no default (DECISIONS, "What a shared map reveals").
   *`choices.md` C3 says "choose at export"; a silent default would publish every
   identifier by omission.*
9. **Command-line spellings of what the previews show only in Python**: an option is the
   parameter's name with dashes (`--max-hops`, `--k`, `--limit`, `--direction`); views
   are `--window START END`, `--edges`, `--nodes`; `avoid=g.top("degree", N)` is
   `--avoid-hubs N`. *One rule instead of a table to memorize; "avoid hubs" is the
   viewer's own label. `--avoid FILE` was dropped: no story needs it.*
10. **Writing things out.** `t.write(path)` is the Python form of `--out`, `g.save(path)`
    of `everygraph export`, both choosing the format from the extension. A graph's file
    name drops every recognized extension; an existing file is replaced only when
    Everygraph can tell it is its own, else `OutputExists` with `out=` and
    `overwrite=True` as ways forward (§ 2.7). Defaults can also come from
    `EVERYGRAPH_MEMORY`, `EVERYGRAPH_THREADS`, `EVERYGRAPH_TEMP_DIR`. *The previews
    show `--out` and `m.export` but no Python call for a table or a graph; hours of
    cached results must not be lost to a name clash; nothing is promised that cannot be
    detected.*
11. **Errors and warnings.** Eight classes under `EverygraphError` (§ 2.7), `UnknownNode`
    a `LookupError` with exit status 6 (DECISIONS, "Exit status for 'no such node'"),
    and one warning class, `TruncatedWarning`; argument and operating-system errors stay
    Python's own, and the command line reports them as `UsageError` (exit status 2)
    and `IoError` (exit status 1) so that `--json` can describe them; a negative or NaN
    weight is `InvalidInput`, a property of the data (exit status 3); reaching a limit
    is never an error, and a clean stop is not one either (in Rust,
    `Outcome::Stopped`). Every class is in the closed enumeration of
    `docs/ux-contract.md` § 6.1, and `ways_forward` holds `Way` objects with the keys
    of its error document. *The previews name one class and #81 two; the rest follow
    from the failures the previews and issues describe. `KeyError` mangles a multi-line
    message.*
12. **Shapes of two results.** `g.components(strong=True)` for strongly connected
    components. `g.neighbors()` returns one row per distinct neighbor with `edges`, the
    summed weight, `first` and `last`; several nodes give one flat table, in input
    order, `limit` per node. *Short names (Choice 13); repeated edges are events, so a
    neighbor is a pair with a count.*
13. **`Hierarchy` is the map's tree of regions** (`m.hierarchy`), with kinds
    `community`, `subdivision`, `aggregate`; the levels of the algorithm are reached
    through `Communities` (`level=`). *`DESIGN.md` D16 separates the two outputs; the
    kinds use the words of `SPEC.md` (#63 says "subcommunity" for the second).*
14. **`eg.read` also takes a table**, and keeps small inputs in memory (below 100 MB †)
    unless `out=` is given. *`api.md` Choice 1 recommends exactly this split; #83 and #95
    ask for tables in.*
15. **Degree columns are `in_degree`, `out_degree`, `degree`**, counting distinct
    neighbors, instead of `in`, `out`, `total` as in `api.md` § 2 and `notebook.html`
    cell 11 (DECISIONS, "Degree columns"). *Departs from the preview: `in` is a keyword of
    the filter grammar and of Python, and a pair is analysed once (`choices.md` A1), so a
    degree counts neighbors. The selection figures of story 5 were re-invented so that
    `degree` is less than `in_degree + out_degree`, as distinct neighbors make it.*
16. **`use_direction=True` on `communities`** (`--use-direction`), instead of
    `directed=True`. *One word, one meaning: `directed` declares what the input is
    (`eg.read`, `ingest --directed`); using the direction is a different request.*
17. **`external_edges` instead of `edges_out`** in the tables of communities and
    regions. *Departs from `api.md` § 3: on a directed graph "out" reads as "outgoing",
    while the column counts the edges with one end inside, either way. Both columns
    count events, like `num_edges`.*
18. **Ingest options `self_loops` and `duplicates`** with the approved defaults
    (`"drop"`, `"events"`, `choices.md` A1, A3), and `--self-loops`, `--duplicates`.
    *#36 builds all three behaviours; a behaviour with no name cannot be asked for.*
19. **Identifiers out** keep the input column's type; hexadecimal text comes back
    lowercase with its prefix (DECISIONS, "Identifiers in results"). Graph order replaces
    the internal numbering in the API: `ids="internal"` and "row *i* is node *i*" were
    removed. *Joins must not fail silently on mixed case; internal numbers would freeze
    an implementation detail.*
20. **Scratch beside the output** (DECISIONS, "Where scratch goes"): plans say
    `61 GB scratch`, without `in /tmp`, and the stop block names `.everygraph-7c1e`.
    *Departs from the previews' `in /tmp` and `rm -r /tmp/…`, which `/tmp` in memory
    would make dangerous; the wording is `docs/ux-contract.md` § 5.4's.*
21. **The map's `share it:` hint is the export command**:
    `everygraph map transfers.eg --out shared.egmap --reveal summary`, and the map's
    `✓` line carries its total time. *Departs from `cli.md` § 1 ("upload transfers.egmap
    and viewer.html"): the working map is full, so uploading it would share every
    identifier, which item 8 exists to prevent; the total time is `docs/ux-contract.md`
    G-11's rule for every `✓` line.*
22. **No "groups".** The map's phase line reads `31,960 · modularity 0.603`, the `info`
    line `31,960 · 2 min 11 s`, the map's text form "9 aggregates holding the other
    31,886", and the plan's steps "move nodes between communities", "merge communities
    and repeat". *Departs from `cli.md`, `terminal.html` and `notebook.html`: one word
    for one thing (DECISIONS, "The word for what community detection finds"); where a
    line has no room, the noun is dropped.*
23. **"deterministic", not "repeatable"**, in `info` (`communities (Leiden,
    deterministic)`). *Departs from `cli.md` § 4: two words for one thing.*
24. **A result cut short is reported in Python by `warnings.warn(…,
    eg.TruncatedWarning)`**, shown by Python with its own prefix (story 3). *Departs
    from the `!` form at a Python prompt in `api.md` § 4: a script must be able to
    silence the warning or make it an error; `docs/ux-contract.md` M-12.* In a notebook
    the approved "Cut short." note of `notebook.html` cell 8 stays on the result's
    card, and the warning is issued as well.
25. **The result document of `--json`** gains `schema`, `view`, `summary`,
    `stored_in`, `truncated` and `meta` beside the keys of `terminal.html`'s example,
    and its schema is closed, under the compatibility rules of `docs/ux-contract.md`
    § 3.6. It is for results that are tables (`run`, `query`); an operation that makes
    a file prints the file document of `docs/ux-contract.md` § 4.7. *#98 asks for
    schemas; a reader must know where the rest of the table is.*
26. **`khop` cut at `limit`** keeps the first `limit` rows by distance, then discovery
    order, and stops there (§ 2.12). *The rows are the same on every run, and the cost
    is bounded by what is returned, which is what `limit` is for.*

**B. Names taken from issue files** (marked ‡; not shown in an approved preview)

| Name | From | Kept because |
|---|---|---|
| `eg.from_edges`, `eg.from_scipy`, `g.save` | #84 | Story 10 needs the first; the names are already in the plan. |
| `g.has_edge`, `g.common_neighbors`, `g.edges_between` | #90 | Named in the plan; they follow the short-name rule. |
| `g.to_networkx`, `g.to_igraph`, `g.to_scipy` | #85 | Story 7 needs the first. |
| `InvalidInput` | #81 | |
| `--threads`, `--temp-dir`, `--seed`, `--deterministic`, `--fast`, `everygraph export`, `everygraph verify` | #98 | |

**C. Names added here** (in neither a preview nor an issue; each is the smallest name
that makes a previewed feature usable)

| Name | Needed for |
|---|---|
| `g.nodes()`, `g.edges()`, `g.columns`, `g.file`, `g.directed`, `g.num_nodes`, `g.info()`, `eg.__version__` | Getting data and metadata out (`api.md` shows only `num_edges`); `info` mirrors the command. `file`, because `g.path` is taken by the path query. |
| `view.base`, `view.definition` | Getting back what a view was made from, and what it prints (§ 3.3). |
| `c.nodes()`, `c.levels`, `c.modularity`, `c.meta`, `Community`, `Community.id`, `Community.nodes()` | The membership table of story 1 has no call in the previews (`c.table()` is per community); `nodes()` follows `m.selection.nodes()`. |
| `Table.write`, `Table.meta`, `Table.columns`, `Table.num_rows`, `Table.truncated`, `Table.head`, `Table.sizes`, `Table.to_arrow` | § 2.5, § 2.8 and the `--out` of the command; `to_arrow()` because users of Polars and DuckDB expect it. |
| `p.found`, `p.hops`, `p.truncated`, `p.meta`, `p.nodes()`, `p.edges()`, `p.on_map()` | The `Path` object the issue asks for; `on_map()` mirrors `Table.on_map()`. |
| `Selection.kind`; `m.file`, `m.levels`, `m.hierarchy`; `Hierarchy.levels`, `Hierarchy.table()`, `Hierarchy.of()`; `Plan.operation`, `Plan.fits`, `Plan.estimate`, `Plan.refusal`, `Plan.to_dict()` | The object model the issue asks for. |
| `progress=`, `temp_dir=`, `out=`, `overwrite=`, `reveal=`, `strong=`, `method=`, `resolution=`, `objective=`, `iterations=`, `use_direction=`, `personalize=`, `damping=`, `tol=`, `max_iter=`, `max_edges=`, `self_loops=`, `duplicates=`, `communities=` of `g.map()`, `seed=`, `weight=` of `degrees` and `to_scipy` | Parameters the implementing issues describe without naming (#36, #47, #48, #57, #58, #63, #81, #85), and `overwrite=` for § 2.7. |
| `InsufficientDisk`, `InvalidExpression`, `UnknownNode`, `Unsupported`, `OutputExists`, `CorruptFile`, `EverygraphError` and its attributes `message`, `why`, `arithmetic`, `ways_forward`, `changed`, `kept`, `machine_memory_bytes`, `to_dict()`, `Way` and its fields `command`, `python`, `note`, `needs_bytes`, `TruncatedWarning` | § 2.7 and § 2.5; the attributes and fields are the keys of the error document of `docs/ux-contract.md` § 6.1. |
| `--window`, `--edges`, `--nodes`, `--avoid-hubs`, `--reveal`, `--deep`, `--no-weight`, `--directed`, `--undirected`, `--strong`, `--use-direction`, `--self-loops`, `--duplicates`, `--overwrite`, `--personalize`, `--precision` of `run pagerank` | Items 9, 16, 18 and 10. |
| `ids=` of `eg.read`; `--ids`, `--topology-only`, `compact` and its `--out` | `design/DECISIONS.md` rows of 2026-11-26: "How identifiers are read: `ids=`", "A structure-only export", "The `compact` command". |
| The result document of § 5.3 (`urn:everygraph:schema:result:1`) and exit status 6 | Item 25 and item 11. |
