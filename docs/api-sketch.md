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
  took     1 s
>>> c.table().head(3)
┌───────────┬───────┬────────────────┬───────────┬───────┐
│ community │ nodes │ internal_edges │ edges_out │ share │
├───────────┼───────┼────────────────┼───────────┼───────┤
│         0 │ 4,917 │         88,214 │     9,480 │ 7.20% │
│         1 │ 4,105 │         71,933 │     8,012 │ 6.01% │
│         2 │ 3,876 │         64,410 │     7,655 │ 5.67% │
└───────────┴───────┴────────────────┴───────────┴───────┘
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
  took     1 s

✓ communities.parquet · 0.4 MB · 68,304 rows · columns: node, community
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
  estimate  3 min 40 s · peak 5.9 GB · 61 GB scratch in /tmp · result 9.3 GB

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

### Story 3 — Neighbours and a shortest path that avoids the hubs

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
┌─────────────┬───────┬────────────┬────────────┬────────────┐
│ neighbor    │ edges │ value      │ first      │ last       │
├─────────────┼───────┼────────────┼────────────┼────────────┤
│ 0x21a3…5549 │   412 │ 18,204.551 │ 2024-01-03 │ 2024-12-30 │
│ 0x3f5c…e2a1 │     7 │      2.100 │ 2024-02-11 │ 2024-02-19 │
│ 0x77e0…c1d4 │     1 │     12.400 │ 2024-03-02 │ 2024-03-02 │
│ 0x9a04…77b3 │    96 │  1,377.020 │ 2024-01-01 │ 2024-11-08 │
│ 0xdac1…1ec7 │ 2,118 │ 90,410.227 │ 2024-01-01 │ 2024-12-31 │
└─────────────┴───────┴────────────┴────────────┴────────────┘
5 rows · cut short at limit=5 of 1,204,877 out-neighbours
>>> near = g.khop(a, k=2, limit=10_000)
! cut short: khop() stopped at its limit of 10,000 nodes,
  part of the way through hop 2. The result is partial
  and marked truncated=True.
  to go further: g.khop("0x28c6…1d60", k=2, limit=100_000)
>>> p
Path · 4 hops · found in 1.3 ms
  0x4f3a…91bc ── value 12.400 · 2024-03-02 ──▶ 0x77e0…c1d4
  0x77e0…c1d4 ── value  3.150 · 2024-03-02 ──▶ 0xb81c…5520
  0xb81c…5520 ── value  3.149 · 2024-03-04 ──▶ 0x02fd…a9e3
  0x02fd…a9e3 ── value  3.100 · 2024-03-09 ──▶ 0x9e12…07aa
  avoided 100 hub nodes · 2 other shortest paths exist: p.alternatives()
```

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

The first two commands print the same tables without borders, and the same "cut short"
message on standard error.

Issues: #90 (point queries, `limit`, `timeout`, `truncated`), #91 (paths, hop limit,
avoid-set, alternatives), #38 (in-edges for directed paths), #92 (the same queries with
the graph on disk), #50 (degrees for `top("degree")`).

### Story 4 — The graph between two dates

```python
march = g.window("2024-03-01", "2024-03-31")
big   = g.filter(edges="value > 10")
both  = march & big

both.num_edges          # 4118306
both
c = both.communities()  # everything that works on a graph works on a view
```

```text
>>> both
View of transfers.eg · 1,204,551 nodes · 4,118,306 edges
  window   2024-03-01 … 2024-03-31
  edges    value > 10
>>> c
Communities of transfers.eg (view) — Leiden, resolution 1.0, deterministic
  4,118 communities · modularity 0.7120 · 0 disconnected
  levels   1,204,551 nodes → 96,410 → 4,118
  largest  88,204 nodes (7.32%) · median 41 · 902 singletons
  note     direction was ignored when grouping
  took     9 s
```

```console
$ everygraph run communities transfers.eg \
      --window 2024-03-01 2024-03-31 --edges "value > 10"
```

It prints the plan block, the `communities` phase line and the same summary.

Issues: #93 (views: filter, compose, mask or materialize), #94 (time windows), #37
(the columns a filter reads), #45 (the planner picks how the view is held), #57.

### Story 5 — A map, and sharing it

```python
m = g.map()            # builds whatever is missing, after printing the plan
m                      # in a notebook: the interactive map
m.highlight(g.pagerank().top(100))
m.selection.nodes()    # what was clicked, back in Python
m.export("share/transfers.egmap", reveal="summary")
```

At a prompt, where no map can be drawn:

```text
>>> m
Map of transfers.eg · 6 zoom levels · 83 regions at the top level
  the 74 largest communities and 9 groups holding the other 31,886
  selection  none
  see it     in a notebook, or: everygraph open transfers.eg
  share it   m.export("transfers.egmap")
>>> m.selection.nodes()          # after selecting community 4 on the map
┌─────────────┬───────────┬───────────┬───────────┬───────────┐
│ node        │ community │ in        │ out       │ total     │
├─────────────┼───────────┼───────────┼───────────┼───────────┤
│ 0x40ec…bbdf │         4 │ 2,118,406 │ 2,004,551 │ 4,122,957 │
│ 0x8eb8…ab28 │         4 │ 1,371,930 │ 1,368,104 │ 2,740,034 │
│ 0x99c9…4be1 │         4 │   904,077 │   688,412 │ 1,592,489 │
│ 0xa3a7…0eec │         4 │   570,345 │   569,902 │ 1,140,247 │
│ 0x3ee1…a585 │         4 │   312,880 │   201,226 │   514,106 │
│ …           │         … │         … │         … │         … │
│ 0xfe21…09c4 │         4 │         1 │         0 │         1 │
└─────────────┴───────────┴───────────┴───────────┴───────────┘
6 of 7,305,118 rows shown · the selection is community 4
```

```console
$ everygraph map transfers.eg
transfers.eg · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges

  machine   16 cores · 61 GB RAM (16 GB free) · NVMe, 1.4 TB free
  budget    8.0 GB memory (default: half of free RAM)
  plan      edges on disk, read in passes — the graph is 1.2× the memory budget
  estimate  about 4 min · peak 5.6 GB · 12 GB scratch in /tmp · result 4.1 GB

  communities  ████████████████  31,960 groups · modularity 0.603   2 min 11 s
  hierarchy    ████████████████  6 zoom levels                            18 s
  layout       ████████████████                                           41 s
  tiles        ████████████████  1,204,118 tiles                     1 min 2 s

✓ transfers.egmap · 4.1 GB
  open it:   everygraph open transfers.eg
  share it:  upload transfers.egmap and viewer.html to any static host
$ everygraph open transfers.eg
Serving on http://127.0.0.1:7461 (this computer only) — opening your browser.
Ctrl-C to stop.
$ everygraph map transfers.eg --out share/transfers.egmap --reveal summary
```

The last command reuses everything already computed (its phase lines say `kept`) and
writes a second map file that holds regions and counts but no identifiers.

Issues: #57 and #60 (communities, also with the edges on disk), #63 (the hierarchy the
map needs), #67 (layout), #68 and #69 (tiles), #75 (export and what it reveals), #74
(`open`), #76 (the notebook widget, selection and highlight), #78 (the caption).

### Story 6 — A result joined with my own table

```python
import duckdb
import polars as pl

labels = pl.read_csv("labels.csv")            # address, label
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
DuckDB read directly (§ 3.4).

```console
$ everygraph run pagerank transfers.eg --out pagerank.parquet
✓ pagerank.parquet · 6.1 GB · 212,418,907 rows · columns: node, pagerank
$ duckdb -c "select l.label, p.pagerank
             from 'pagerank.parquet' p join 'labels.csv' l on p.node = l.address
             order by p.pagerank desc limit 10"
```

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
    g.subgraph(c[4]).save("community-4.graphml")           hand over a file

Nothing was changed.
```

```console
$ everygraph export transfers.eg --window 2024-03-01 2024-03-02 \
      --edges "value > 1000" --out big-transfers.graphml
✓ big-transfers.graphml · 3.4 MB · 9,204 nodes · 12,877 edges
$ python -c "import networkx as nx
G = nx.read_graphml('big-transfers.graphml')"
```

On the command line the hand-over is a file, and the piece is chosen by a filter or a
time window; picking a community or a neighbourhood as the piece is Python-only in 0.1
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
    g.communities(precision="low")    needs 3.4 GB; slightly lower quality

Nothing was changed.
```

With enough memory, `g.explain("communities")` shows how it would run:

```text
>>> g.explain("communities")
Plan for communities() on transfers.eg — Leiden, resolution 1.0, deterministic
  fits     peak 5.6 GB of 8.0 GB budget

  1  keep per-node state         memory            5.1 GB
  2  move nodes between groups   edges from disk   0.5 GB   about 12 passes
  3  merge groups and repeat     memory          < 0.4 GB   4 levels
  4  write the result            disk                   —

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
    --precision low         needs 3.4 GB; slightly lower quality

Nothing was changed.
$ echo $?
4
```

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

  kept      reading · 31 GB of scratch in /tmp/everygraph-7c1e
  dropped   identifiers at 61% — that phase starts again from its beginning
  written   nothing: there is no transfers.eg yet

  continue:  run the same call again
  discard:   rm -r /tmp/everygraph-7c1e
KeyboardInterrupt
```

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

  kept      reading · 31 GB of scratch in /tmp/everygraph-7c1e
  dropped   identifiers at 61% — that phase starts again from its beginning
  written   nothing: there is no transfers.eg yet

  continue:  run the same command again
  discard:   rm -r /tmp/everygraph-7c1e
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

(The first transcript is shown from the phase lines onward; the summary and plan of
story 2 come before them.) The behaviour is identical in a notebook, where the last
line reads `run the cell again` (`docs/ux-contract.md` § 5).

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
  edges     undirected
  ids       integers
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
5 rows
```

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
| "communities" are what the algorithm finds; "regions" are the areas of a map. | `g.communities()`, `m.hierarchy` |
| A part of a graph is a "view". | `g.filter()`, `g.window()`, `g.subgraph()` |
| "budget" means memory and nothing else; a cap on a traversal is `limit` (nodes) and `timeout` (seconds). | `memory="8GB"`, `limit=10_000`, `timeout=10` |
| Files: `.eg` is a graph, `.egmap` is a shareable map. | `transfers.eg`, `transfers.egmap` |
| Everything after the positional subject of a call is keyword-only. | `g.khop(a, k=2, limit=10_000)` |
| A command-line option is the parameter's name with dashes. | `max_hops=5` ↔ `--max-hops 5` |
| Column names of result tables are lowercase words with underscores, are part of the API, and are what the command prints as headers. | `node`, `pagerank`, `internal_edges` |

### 2.2 Identifiers in and out

- **In.** A node is named by its identifier as it appears in the input: a `str`, an
  `int`, or `bytes`. Hexadecimal identifiers are matched without regard to case.
  Shortened forms (`0x28c6…1d60`) are for display and are never accepted.
- **Out.** Every result carries the original identifiers, in a column called `node`
  (or `neighbor`, `source`, `target` where a row is about an edge), with the type the
  identifier had in the input: text, integer, or fixed-width bytes shown as text.
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
- **Internal numbers.** The engine numbers nodes densely from 0. The numbering is fixed
  for a given graph file. It is visible in two places only: the row order of
  `g.nodes()` (row *i* is node *i*), and exports made with `ids="internal"`, where
  `node` is that number instead of the identifier — smaller and faster, for people who
  join by position.

### 2.3 Units and numbers

| Quantity | In arguments | In results and JSON | Printed |
|---|---|---|---|
| Memory, sizes | `"8GB"`, `"500MB"`, `"1.5GB"`; units `kB MB GB TB` (powers of 10) and `KiB MiB GiB TiB` (powers of 2), case-insensitive; an `int` is bytes | bytes (`*_bytes`) | `8.0 GB` (GB = 10⁹ bytes) |
| Time limits and durations | seconds, `int` or `float` | seconds (`*_seconds`) | `38 s`, `1 min 41 s`, `1.3 ms` |
| Instants | ISO 8601 text, `datetime.date`, `datetime.datetime`, `numpy.datetime64`; UTC unless an offset is given | Arrow timestamp, UTC | `2024-03-02` |
| Counts | `int` | unsigned integers | exact, with separators: `1,830,397,917` |
| Scores | — | `float64` (`float32` with `precision="low"`) | as many decimals as distinguish the rows shown |

Printed numbers follow `docs/ux-contract.md` § 2.6 on every surface. Numbers inside
tables handed to other tools are plain values with no formatting.

### 2.4 Parameters that mean the same everywhere

| Parameter | Command line | Type | Default | Taken by | Meaning |
|---|---|---|---|---|---|
| `memory` | `--memory SIZE` | size (§ 2.3) | half of the memory free when the operation is planned | everything that is planned | The memory budget: the most memory the engine may own. Pages of the graph file that the operating system maps from disk are not counted, because it can drop them at any moment. An operation whose per-node state cannot fit is refused before it starts (`InsufficientBudget`); an operation that starts stays inside the budget. The budget never changes a result. |
| `threads` | `--threads N` | `int` ≥ 1 | the physical cores the process may use | everything that is planned | The most worker threads to use. The planner may use fewer (a small input, a kernel that does not scale) and, only when `threads` is not given, may add the second hardware thread of each core for kernels that gain from it (`docs/DESIGN.md` D13). A deterministic result never depends on it. |
| `temp_dir` | `--temp-dir DIR` | path | the system's temporary directory | operations that need scratch | Where scratch files go. |
| `deterministic` | `--deterministic` / `--fast` | `bool` | `True` | `communities`, `map` | `True`: the same input, parameters and `seed` give the identical result, to the bit, for any number of threads, any memory budget and any way the planner runs it. `False` is the faster mode: the result is as good on average but not repeatable. It is labelled `fast` wherever the result is printed or stored, and is never returned in place of a deterministic one. Every other operation is always deterministic and does not take the parameter. |
| `seed` | `--seed N` | `int` ≥ 0 | `0` | `communities`, `map` | Selects one of the equally valid answers of a randomized method. Same seed, same answer (when deterministic). |
| `precision` | `--precision full\|low` | `"full"` or `"low"` | `"full"` | operations that have a lower-memory variant | `"low"` keeps per-node state in 32 bits instead of 64 where the operation supports it, roughly halving the state at a small cost in quality. The result is labelled `precision low` and is never returned in place of a `full` one. Asking for `"low"` where no such variant exists raises `Unsupported`. |
| `weight` | `--weight NAME` / `--no-weight` | `bool` or column name | `True` | `pagerank`, `communities`, `degrees`, `to_scipy` | How much each pair of connected nodes counts. `True`: the sum of the graph's weight column over the pair's edges if a weight column was declared when the graph was read, otherwise the number of edges between the pair. A name: the sum of that numeric edge column. `False`: every connected pair counts 1. What was used is recorded in the result. (`path` differs: § 3.7.) |
| `limit` | `--limit N` | `int` ≥ 1, nodes | per call (§ 3.2) † | `neighbors`, `khop`, `path`, `common_neighbors`, `edges_between` | The most nodes a traversal may return or visit. Reaching it ends the call with a partial result marked `truncated` (§ 2.5), not with an error. |
| `timeout` | `--timeout SECONDS` | seconds | per call (§ 3.2) † | `khop`, `path` | The most time a traversal may take. Same outcome as `limit`. |
| `dry_run` | `--dry-run` | `bool` | `False` | everything that is planned | Plan only: return the `Plan` (§ 3.9), run nothing, write nothing. |
| `progress` | — | `bool` or a function of one `dict` | `True` | everything that is planned | `True`: print the plan and progress (`docs/ux-contract.md` § 2.1). `False`: print nothing. A function: receive the events of `docs/ux-contract.md` § 4.6 instead. |

"Everything that is planned" is: `eg.read`, `eg.from_edges`, every whole-graph
algorithm, `g.map()`, `g.save()` and `m.export()`. The four parameters `memory`,
`threads`, `dry_run` and `progress` are written `**run` in the signatures of § 3.

**Where a default comes from**, first match wins: the argument of the call; the value
given to `eg.read` or `eg.open` when the graph was obtained; the environment
(`EVERYGRAPH_MEMORY`, `EVERYGRAPH_THREADS`, `EVERYGRAPH_TEMP_DIR`); the built-in
default. The plan always says which (`docs/ux-contract.md` § 3.4, the `budget` line).

### 2.5 Results that are partial or approximate

Decided (`design/preview/choices.md` E1): always marked in the result itself, never only
in the documentation.

| | Truncated | Approximate |
|---|---|---|
| Happens when | a traversal reached its `limit` or `timeout` | a sampled method was used (no method in the previews is; see § 7) |
| Attribute | `.truncated` is `True` | `.approximate` is `True` |
| Detail | `meta["truncated"]`: `by` (`"limit"` or `"timeout"`), the value of the limit, `rows` returned, `where` it stopped (`"hop 2"`), and `total` if it is known | `meta["approximate"]`: `column`, `error` (the bound, in the column's unit), `confidence`, `samples` |
| Printed with the table | in the footer: `cut short at limit=10,000, in hop 2` | in the footer: `approximate: betweenness ± 0.002 (95%, 4,096 samples)` |
| Printed when it happens | the "cut short" warning (`docs/ux-contract.md` § 6.2) | nothing more |
| In `--json` and in Arrow | `"truncated": {…}` in the document; the same object in the schema metadata of the Arrow stream, so it survives into Parquet | `"approximate": {…}`, likewise |
| Exit status | 0 | 0 |

A table of which only some rows are *displayed* is not truncated: its footer says
`6 of 31,960 rows shown` and the object holds every row.

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
  last row. The footer says how many rows are shown of how many, how they are ordered,
  and anything from § 2.5. If the grid would pass 80 columns, identifiers are shortened
  in the middle — on display only.
- On the command line the same table has no borders: columns aligned with spaces on a
  terminal, tab-separated with a header row when piped, one document with `--json`
  (§ 5.3).
- Nothing prints by itself except through `progress` (§ 2.4): an object prints when the
  prompt or the notebook displays it.

### 2.7 Errors

All errors raised by the engine derive from `eg.EverygraphError`. Each carries the whole
message as its text, in the pattern of `docs/ux-contract.md` § 6.1 — what, why with the
arithmetic, the ways forward as code that can be copied, and what state things were left
in — and the same content as attributes.

| Exception | Also a | Raised when | Particular attributes | Exit status |
|---|---|---|---|---|
| `InsufficientBudget` | — | The per-node state of an operation does not fit the memory budget. Always before any work. | `step`, `needs_bytes`, `budget_bytes`, `smallest_budget_bytes` | 4 |
| `InsufficientDisk` | — | The scratch or output directory lacks room for the plan. Before any work; or during a run if the disk fills. | `step`, `needs_bytes`, `available_bytes`, `path` | 4 |
| `InvalidInput` | — | The input cannot be read as described: a row with the wrong number of fields, a value that is not of the column's type, a missing column, an unknown format. | `problems` (each with `file`, `line`, `problem`, `excerpt`), `invalid_rows`, `rows_checked` | 3 |
| `InvalidExpression` | `ValueError` | The text of a filter does not parse or names a column the graph lacks. Raised by `g.filter()` itself, not later. | `expression`, `position` | 2 |
| `UnknownNode` | `KeyError` | A call about one node names an identifier that is not in the graph. | `node` | 2 |
| `Unsupported` | — | The operation is not defined for this graph (`window` without a time column, `strong=True` on an undirected graph), has no such variant (`precision="low"`), or is above a size guard (`to_networkx`). | `operation`, `reason` | 2 |
| `CorruptFile` | — | A graph or map file fails its integrity checks, or was written by a format this version cannot read. | `path`, `section` | 5 |

Common attributes of every `EverygraphError`: `message` (the first sentence),
`ways_forward` (a list of pairs: code, note), `changed` (`False` when nothing was
written), and `to_dict()`, which is the error document of `docs/ux-contract.md` § 6.1.

Outside the taxonomy on purpose:

- A wrong argument — an unknown keyword, `k=-1`, `direction="sideways"` — is Python's
  own `TypeError` or `ValueError`.
- Errors of the operating system pass through as they are: `FileNotFoundError`,
  `PermissionError`, `FileExistsError` (an output path that holds something Everygraph
  did not write is never overwritten).
- `KeyboardInterrupt` is never wrapped (`docs/ux-contract.md` § 5.2).
- Reaching a `limit` or a `timeout` is not an error (§ 2.5).

The exact text of a refusal, as an example of the pattern, is in story 8.

### 2.8 What is kept, and where

| What | Where | For how long |
|---|---|---|
| The graph | `NAME.eg`, beside the input, when `eg.read` is given a file of at least 100 MB † or `out=` is given; otherwise in memory | until deleted |
| The result of every whole-graph algorithm: PageRank, components, core numbers, triangles, degrees, communities with their levels, the map's hierarchy and layout | inside the graph file, with a record of the parameters that produced it (#39 decides between the file itself and a companion beside it) | until the file is compacted or deleted |
| The same, for a view | inside the graph file, under the view's definition | likewise |
| The same, for a graph in memory | in memory | while the graph object lives |
| The map | `NAME.egmap`, beside the graph | until deleted |
| Scratch of a running or stopped job | a directory `everygraph-ID` under `temp_dir` | removed on success (`docs/ux-contract.md` § 5) |
| Point queries, plans, tables derived in Python (`.top()`, `.head()`) | nowhere | — |

- A result is looked up before it is computed. The key is the graph's content, the
  view's definition, the algorithm and its version, every parameter that can change the
  result, `seed`, `precision`, and whether it is deterministic. `memory`, `threads` and
  `progress` are not part of the key.
- A hit is not silent: the phase is shown as `kept`, `meta["cached"]` is `True`, and the
  command prints `full result: transfers.eg (cached)`.
- There is no hidden cache directory in a home folder (`api.md` Choice 1).
- If the graph file cannot be written (a read-only medium), results are kept for the
  life of the graph object and a warning says so once.
- `eg.read` on an input whose `.eg` already exists, was built from the same bytes with
  the same parameters and is intact, opens it instead of building it again, and says so.

### 2.9 Threads and processes

- `Graph`, `View`, every table, `Communities`, `Path` and `Plan` are immutable. They can
  be shared between threads and used from several at once without locks.
- Every call that computes releases the interpreter lock for its whole duration, so
  point queries from many Python threads run in parallel.
- A `Map` shown in a notebook has state (its selection, its highlights) and belongs to
  the thread that displays it.
- Several processes may read one graph file at once. Adding a result to it is done by
  one process at a time, under a lock on the file; a second writer waits, and its
  progress line says what it is waiting for. A reader never sees a half-written result
  (`docs/ux-contract.md` § 5.2, C-1).
- A function passed as `progress` is called on the calling thread, never concurrently.

### 2.10 What is stable in 0.1

Promised not to change without a deprecation in a later 0.x release:

- the Python names, parameters and defaults of § 3 that are not marked † or ‡;
- the column names and types of result tables (§ 3.4);
- the exception classes of § 2.7 and their attributes;
- the commands, options and exit statuses of § 5;
- the JSON documents of `docs/ux-contract.md` (plan, events, errors), under their
  `schema` number;
- the determinism guarantee: within one version of Everygraph, the same input,
  parameters and seed give identical results on every machine the release supports,
  with the exceptions documented by #54.

Not promised:

- the exact wording and layout of printed text (it is covered by golden tests, and it
  may still improve);
- values marked †, and names marked ‡ (taken from an issue file, not shown in an approved
  preview);
- identical results across versions of Everygraph (an algorithm's version is part of the
  cache key, so an upgrade recomputes rather than mixing);
- the Rust API of § 4, which follows Rust's rules for 0.x crates;
- the `.eg` and `.egmap` formats before their specifications say so (#28, #68).

---
## 3. The object model (Python)

Ten kinds of object. The signatures are written as Python stubs; they are the
specification of names, parameter order, types and defaults. In them:

```python
Node      = str | int | bytes                # one identifier (§ 2.2)
Nodes     = ...                              # any node set of § 2.2
Size      = str | int                        # "8GB", or bytes (§ 2.3)
Time      = str | datetime.date | datetime.datetime | numpy.datetime64
Direction = Literal["out", "in", "both"]
Precision = Literal["full", "low"]
TableLike = ...   # anything with the Arrow PyCapsule stream interface:
                  # pyarrow, Polars, pandas, a DuckDB relation, an eg.Table

class Run(TypedDict, total=False):           # written **run below (§ 2.4)
    memory: Size
    threads: int
    dry_run: bool
    progress: bool | Callable[[dict], None]
```

Any call that takes `**run` returns a `Plan` instead of its usual result when
`dry_run=True`. ‡ marks a name taken from an issue file rather than an approved preview;
† marks a placeholder value.

| Object | What it is | Obtained from |
|---|---|---|
| `Graph` | An immutable graph: in a `.eg` file, or in memory. | `eg.read`, `eg.open`, `eg.from_edges` |
| `View` | Part of a graph, defined by a filter, a time window or a node set. Behaves as a graph. | `g.filter`, `g.window`, `g.as_of`, `g.subgraph`, `view & view` |
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
    out: str | os.PathLike | None = None,
    temp_dir: str | os.PathLike | None = None,
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
    **run: Unpack[Run],
) -> Graph: ...

def from_scipy(matrix, *, directed: bool | None = None) -> Graph: ...  # ‡ #84

__version__: str
```

| Function | Details | Implemented by |
|---|---|---|
| `eg.read` | Reads a file or a table of edges and returns a graph. **`data`**: a path — text or CSV, optionally `.gz` or `.zst` (#32); Parquet (#33); GraphML, GEXF, Matrix Market (#85); a `.eg` file, which is simply opened — or any table (`TableLike`), read without copying (#83, #95). **`source`, `target`**: column names; when omitted, the columns named `from`/`to` or `source`/`target`, else the first two. **`weight`, `time`**: optional column names; they become the graph's weight and time columns. Other columns are ignored, and listed. **`directed`**: when `None`, directed if the columns are named like a direction (`from`/`to`, `source`/`target`), undirected otherwise; the summary says which and why. **`out`**: where to write the `.eg`. Default: beside the input, same name, when the input is a file of at least 100 MB †; smaller inputs and tables stay in memory. Repeated edges are kept as separate events; self-loops are dropped and counted (#36). `memory` and `threads` given here become the defaults of the returned graph. Prints the summary always, and the plan and progress when long (`docs/ux-contract.md` § 3, § 4). | #81; #32, #33, #34, #35, #36, #40, #83, #84, #85 |
| `eg.open` | Opens a `.eg` file. Reads its header and its table of contents only, whatever the file's size (#28). Raises `CorruptFile` if the quick integrity check fails (#41). | #81; #28, #41 |
| `eg.from_edges` ‡ | A graph from two equal-length arrays of identifiers (NumPy or Arrow), kept in memory. | #84 |
| `eg.from_scipy` ‡ | A graph from a SciPy sparse matrix; `directed=None` means directed unless the matrix is symmetric. | #84 |

### 3.2 `Graph`

A graph is immutable. It is either backed by a `.eg` file — opening it is instant and it
may be larger than memory — or held in memory. Nodes have identifiers; edges may carry a
weight, a time and other columns; repeated edges between the same two nodes are separate
events (`docs/spec/graph-model.md`, to be written by #36, defines what each algorithm
does with them).

```python
class Graph:
    num_nodes: int
    num_edges: int                    # stored edges: every event counts
    directed: bool
    file: pathlib.Path | None         # None for a graph in memory
    columns: Table                    # name, on, type, role

    def info(self) -> Info: ...
    def nodes(self) -> Table: ...
    def edges(self) -> Table: ...
    def save(self, path: str | os.PathLike, **run: Unpack[Run]
             ) -> pathlib.Path: ...                                # ‡ #84

    # --- questions about the whole graph ---------------------------------
    def pagerank(self, *, damping: float = 0.85, tol: float = 1e-6,
                 max_iter: int = 100, weight: bool | str = True,
                 personalize: Nodes | None = None,
                 precision: Precision = "full",
                 **run: Unpack[Run]) -> Table: ...
    def components(self, *, strong: bool = False,
                   **run: Unpack[Run]) -> Table: ...
    def kcore(self, **run: Unpack[Run]) -> Table: ...
    def triangles(self, **run: Unpack[Run]) -> Table: ...
    def degrees(self, nodes: Nodes | None = None, *,
                weight: bool | str = True) -> Table: ...
    def bfs(self, source: Node, *, direction: Direction = "out",
            max_hops: int | None = None, **run: Unpack[Run]) -> Table: ...
    def top(self, by: str, n: int = 10, **run: Unpack[Run]) -> Table: ...
    def communities(self, *, method: str = "leiden",
                    resolution: float = 1.0,
                    objective: str = "modularity",
                    iterations: int = 1,
                    weight: bool | str = True,
                    directed: bool = False,
                    deterministic: bool = True, seed: int = 0,
                    precision: Precision = "full",
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
    def as_of(self, time: Time) -> View: ...                       # ‡ #94
    def subgraph(self, nodes: Nodes) -> View: ...

    # --- to small-graph tools ---------------------------------------------
    def to_networkx(self, *, events: bool = False,
                    max_edges: int = 1_000_000): ...               # ‡ #85
    def to_igraph(self, *, max_edges: int = 1_000_000): ...        # ‡ #85
    def to_scipy(self, *, weight: bool | str = True): ...          # ‡ #85
```

The values of `limit`, `timeout`, `max_edges`, `tol` and `max_iter` are †.

**The whole graph.** Each of these is planned, prints its plan and one progress line
when long, is deterministic, stores its result in the graph file and returns it from
there the next time (§ 2.8). Each returns one row per node of the graph or view, in the
graph's node order.

| Method | Returns (columns) | Notes | Implemented by |
|---|---|---|---|
| `pagerank()` | `node`, `pagerank` | Scores sum to 1. Stops when the sum of absolute changes between two iterations is below `tol` — not scaled by the number of nodes, unlike NetworkX — or after `max_iter` iterations; `meta` has `iterations` and `converged`. `personalize`: restart at these nodes only. Has a `precision="low"` variant. | #47 |
| `components()` | `node`, `component` | Weakly connected components; `strong=True` for strongly connected ones (directed graphs only). Components are numbered from 0 by decreasing size. `.sizes()` gives `component`, `nodes`. | #48 |
| `kcore()` | `node`, `core` | The core number of each node. `.sizes()` gives `core`, `nodes`. | #50 |
| `triangles()` | `node`, `triangles`, `clustering` | Triangles through each node and its local clustering coefficient. `meta` has `triangles_total` and `transitivity`. Direction is ignored. | #51 |
| `degrees(nodes=None)` | `node`, `in`, `out`, `total`, `weighted` | For every node, or for the nodes given. Undirected graphs have `total` and `weighted` only; `weighted` is present when the graph has a weight. Computed when the graph is built, so never long. | #50, #90 |
| `bfs(source)` | `node`, `distance`, `parent` | Hops from `source` to every node; null where unreachable. `max_hops` stops early. | #49 |
| `top(by, n)` | `node`, the measure, `rank` | The `n` nodes with the highest value of a per-node measure: `"degree"` (the `total` of `degrees()`), `"in"`, `"out"`, `"weighted"`, `"pagerank"`, `"core"`, `"triangles"`, `"clustering"`. Computes the measure with default parameters if it is not stored. Same as `g.pagerank().top(n)` and so on. | #81 |
| `communities()` | a `Communities` (§ 3.5) | `method`: `"leiden"`, or `"label_propagation"` — a quick preview, labelled as one (#65). `resolution`: larger gives more, smaller communities. `objective`: `"modularity"` or `"cpm"` (#58). `iterations`: 1 by default; 3 is the setting #57 calls "quality". `directed=False`: direction is ignored when grouping, and the result says so (`choices.md` A2); `True` uses it (#58). | #57, #58, #60, #62, #65 |
| `map()` | a `Map` (§ 3.8) | Does everything that is missing — communities, hierarchy, layout, tiles — after printing the plan, and writes `NAME.egmap` beside the graph. `communities=`: draw these instead of the default ones. | #63, #67, #69, #76 |
| `explain(operation, **parameters)` | a `Plan` (§ 3.9) | `g.explain("communities", memory="4GB")` is `g.communities(memory="4GB", dry_run=True)`. `operation` is the name of any method that takes `**run`. Never raises a refusal: a plan that does not fit says so. | #45 |

**Particular nodes.** These are answered at once: no plan, no progress, nothing stored.
Each takes a `limit`, and where a search could run long a `timeout`; reaching either
returns what was found, marked `truncated` (§ 2.5).

| Method | Returns (columns) | Notes | Implemented by |
|---|---|---|---|
| `neighbors(nodes)` | `neighbor`, `edges`, then the weight column under its own name, `first`, `last` | One row per distinct neighbour: how many edges join the pair, the sum of their weight, the first and last time. The last three are present when the graph has a weight and a time column. With several nodes, a leading `node` column says whose neighbour each row is. Rows are in the graph's node order. `limit` counts rows. | #90 |
| `khop(node, k=…)` | `node`, `distance` | Every node within `k` hops, nearest first. `limit` counts nodes returned. | #90 |
| `path(source, target)` | a `Path` (§ 3.7) | A shortest path by number of hops; with `weight="column"`, by the smallest sum of that column (a cost — this is not the `weight` of § 2.4). `max_hops`: give up beyond this length. `avoid`: nodes the path may not pass through. `limit` counts nodes the search may visit. On a view, the path stays inside the view. | #91, #38 |
| `has_edge(source, target)` ‡ | `bool` | | #90 |
| `common_neighbors(a, b)` ‡ | `neighbor`, `edges_a`, `edges_b` | | #90 |
| `edges_between(a, b)` ‡ | `source`, `target`, `edges`, weight, `first`, `last` | Pairs joined by at least one edge, one end in each set. | #90 |

**Out of the graph.**

| Member | Returns | Notes | Implemented by |
|---|---|---|---|
| `num_nodes`, `num_edges`, `directed`, `file` | | On a view, the counts are the view's. | #81 |
| `columns` | `name`, `on`, `type`, `role` | The columns of the graph: `on` is `nodes` or `edges`; `role` is `source`, `target`, `weight`, `time` or empty. These are the names a filter may use. | #37 |
| `info()` | an `Info` | A mapping whose text form is exactly what `everygraph info` prints (below): the file, its counts and columns, and everything computed so far with its parameters, time and date. | #39, #98 |
| `nodes()` | `node`, then node columns | One row per node, in the graph's node order. | #81, #83 |
| `edges()` | `source`, `target`, then edge columns | One row per stored edge. Streamed: it is as large as the graph. | #81, #83 |
| `save(path)` ‡ | the path | Writes the graph or view as a new file; the format follows the extension: `.eg`; `.parquet`, `.csv` (the edge table); `.graphml`, `.gexf`, `.mtx`. Planned like any long operation. | #84, #85 |
| `to_networkx()` ‡ | `networkx.Graph` or `DiGraph` | Each connected pair becomes one edge with attributes `edges` (how many) and the summed weight; `events=True` gives a `MultiGraph`/`MultiDiGraph` with one edge per event. Nodes are the original identifiers and carry `community` and the map position `x`, `y` when those are computed. Above `max_edges` it raises `Unsupported` with the ways forward (story 7). | #85, #87 |
| `to_igraph()`, `to_scipy()` ‡ | an `igraph.Graph`; a SciPy sparse matrix | Same guard and same projection. | #85 |

**What it prints.** A graph in a file prints the first block of `everygraph info`;
`g.info()` prints all of it. A graph in memory prints as in story 10. The notebook card
is "the graph card" of `notebook.html` (name, kind, the two counts with the user's
nouns).

```text
>>> g.info()
transfers.eg · 9.3 GB · created 2026-11-24 from transfers.parquet

  nodes       212,418,907
  edges     1,830,397,917 · directed, with duplicates as events
  ids       hex addresses
  time      2024-01-01 … 2024-12-31
  columns   value (weight), block_time (time)

  computed so far
    degrees                           at ingest
    communities (Leiden, repeatable)  31,960 groups · 2 min 11 s · 2026-11-24
    pagerank                          41 s · 2026-11-24
    map                               transfers.egmap · 4.1 GB
```

### 3.3 `View`

A view is part of a graph. It is a `Graph` — `isinstance(view, eg.Graph)` holds, and
every method of § 3.2 works on it — defined by text, so it costs nothing to make and can
be stored by its definition. Whether it is evaluated through a mask or materialized as a
smaller graph is the planner's choice (`docs/DESIGN.md` D10) and never changes a result.

```python
class View(Graph):
    base: Graph                       # the graph it is part of
    definition: dict                  # window, edges, nodes: what is printed

    def __and__(self, other: View) -> View: ...
```

| Made by | Keeps | Notes | Implemented by |
|---|---|---|---|
| `g.filter(edges="value > 10")` | The edges for which the expression is true, and the nodes that still have an edge. | The expression language is below. | #93, #37 |
| `g.filter(nodes="in > 100")` | The nodes for which it is true, and the edges with both ends among them. | Names available in 0.1: the graph's node columns, and `in`, `out`, `total`. | #93 |
| `g.window(start, end)` | The edges whose time is from `start` to `end`, both included, and the nodes that still have an edge. | A date without a time, given as `end`, includes that whole day. Either bound may be `None`. Needs a time column, else `Unsupported`. | #94 |
| `g.as_of(time)` ‡ | The edges up to and including `time`. | `g.window(None, time)`. | #94 |
| `g.subgraph(nodes)` | The given nodes and the edges with both ends among them. | Accepts any node set (§ 2.2): `g.subgraph(c[0])`, `g.subgraph(m.selection)`, `g.subgraph(g.khop(a, k=2))`. | #87, #93 |
| `a & b` | What is in both. | Both must be views of the same graph. A chain is the same thing: `g.window(…).filter(…)`. | #93 |

A filter is short text (`api.md` Choice 9), checked when `filter()` is called:

```text
expression := term { ("and" | "or") term }        "and" binds tighter
term       := ["not"] ( comparison | "(" expression ")" )
comparison := column op value
            | column ["not"] "in" "(" value { "," value } ")"
            | column "is" ["not"] "null"
op         := "==" | "!=" | "<" | "<=" | ">" | ">="
column     := a name from g.columns, or `any name in backquotes`
value      := a number | 'text' | "text" | true | false
```

A time column is compared with text in ISO 8601 form: `block_time >= '2024-03-01'`.
There is no arithmetic and there are no functions in 0.1. A mistake raises
`InvalidExpression`, whose text shows the expression with a caret under the place.

A view prints as in story 4: one headline with its counts, then one line for each part
of its definition. Results computed on a view say `(view)` after the graph's name.

### 3.4 `Table`

Every algorithm and query returns a table. It **is** an Arrow table: it implements the
Arrow PyCapsule interface, so `pl.from_arrow(t)`, `pa.table(t)`, `t.to_pandas()` and
`duckdb.sql("select … from t")` read it directly, the numeric columns without a copy
(#83, #95). It is a handle on a stored result, so a table with a row for every node of a
large graph costs no memory until it is handed over, and then it is streamed in batches.
The identifier column is produced as the table is handed over and costs what the
identifiers weigh; `ids="internal"` avoids that.

```python
class Table:
    columns: list[str]
    num_rows: int
    truncated: bool
    approximate: bool
    meta: Mapping[str, object]

    def head(self, n: int = 5) -> Table: ...
    def top(self, n: int = 10, by: str | None = None) -> Table: ...
    def sizes(self) -> Table: ...
    def on_map(self) -> Map: ...
    def write(self, path: str | os.PathLike, *,
              ids: Literal["original", "internal"] = "original"
              ) -> pathlib.Path: ...
    def to_arrow(self, *, ids: Literal["original", "internal"] = "original"
                 ) -> "pyarrow.Table": ...
    def to_pandas(self, *, ids: Literal["original", "internal"] = "original"
                  ) -> "pandas.DataFrame": ...

    def __len__(self) -> int: ...
    def __arrow_c_schema__(self) -> object: ...
    def __arrow_c_stream__(self, requested_schema: object | None = None
                           ) -> object: ...
```

| Member | Returns | Notes | Implemented by |
|---|---|---|---|
| `columns`, `num_rows`, `len(t)` | | | #81 |
| `truncated`, `approximate` | `bool` | § 2.5. | #90, #52 |
| `meta` | a read-only mapping | Always: `operation`, `parameters`, `graph`, `view`, `engine`, `deterministic`, `seed`, `precision`, `weight`, `took_seconds`, `peak_memory_bytes`, `cached`, `plan`, `truncated`, `approximate`. Plus what the operation adds (`iterations`, `modularity`, `triangles_total`, …). Exported as the schema metadata of the Arrow stream under the key `everygraph`. | #45, #39 |
| `head(n)` | `Table` | The first `n` rows. | #81 |
| `top(n, by)` | `Table` | The `n` rows with the largest value of `by` (default: the table's last numeric column), largest first, with a `rank` column added. | #47, #81 |
| `sizes()` | `Table` | For a table with a grouping column (`component`, `core`, `community`): one row per group with its number of nodes, largest first. | #48, #50 |
| `on_map()` | `Map` | Shows these nodes on the graph's map: `g.map().highlight(t)`, and returns the map. Builds the map if it does not exist, after printing the plan. | #76 |
| `write(path)` | the path | Format from the extension: `.parquet`, `.csv`, `.tsv`, `.arrow`. The Python form of `--out`. | #83, #85 |
| `to_arrow()`, `to_pandas()` | | Conveniences that need `pyarrow`, respectively `pandas`, installed; neither is a dependency of `everygraph`. | #83 |
| `ids="internal"` | | Gives `node` as the engine's own number (§ 2.2) instead of the identifier. | #81 |

The columns of every table, which are part of the API:

| From | Columns and types |
|---|---|
| `pagerank()` | `node` · `pagerank` float64 |
| `components()` | `node` · `component` uint32 |
| `kcore()` | `node` · `core` uint32 |
| `triangles()` | `node` · `triangles` uint64 · `clustering` float64 |
| `degrees()` | `node` · `in` uint64 · `out` uint64 · `total` uint64 · `weighted` float64 or decimal |
| `bfs()` | `node` · `distance` uint32, nullable · `parent` identifier, nullable |
| `top()`, `.top()` | the source table's columns · `rank` uint32 |
| `.sizes()` | the grouping column · `nodes` uint64 |
| `neighbors()` | [`node`] · `neighbor` · `edges` uint64 · [weight under its name] · [`first`, `last` timestamp] |
| `khop()` | `node` · `distance` uint32 |
| `c.nodes()` | `node` · `community` uint32 |
| `c.table()` | `community` uint32 · `nodes` uint64 · `internal_edges` uint64 · `edges_out` uint64 · `share` float64 · [`parent` uint32] |
| `c.of()` | [`node`] · `level` uint32 · `community` uint32 · `nodes` uint64 |
| `m.selection.nodes()` | `node` · `community` uint32 · `in` · `out` · `total` uint64 |
| `m.hierarchy.table()` | `level` · `region` · `parent` uint32 · `kind` text · `name` text · `nodes` · `internal_edges` · `edges_out` uint64 |
| `g.nodes()` | `node` · node columns |
| `g.edges()` | `source` · `target` · edge columns |
| `g.columns` | `name` · `on` · `type` · `role`, all text |

`share` is a fraction from 0 to 1 in the data and a percentage when printed. A weight
that was a decimal or an integer in the input stays exact (#36).

The text form is described in § 2.6 and shown in the stories; the notebook form is the
table card of `notebook.html` (cells 5, 6 and 11), with the same footer.

### 3.5 `Communities` and `Community`

What `g.communities()` returns: the partition of the nodes into communities, the coarser
and finer levels the method went through, and exact counts for every community at every
level.

```python
class Communities:
    modularity: float
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
    level: int
    def nodes(self) -> Table: ...                 # node
    def __len__(self) -> int: ...                 # number of nodes
```

| Member | Returns | Notes | Implemented by |
|---|---|---|---|
| `len(c)`, `c.modularity`, `c.levels` | | The number of communities in the partition, its modularity, and how many levels there are. | #57, #62 |
| `c.table(level=1)` | one row per community | Level 1 is the partition — the answer. Level 2 and beyond are the finer groupings the method built it from, each row with its `parent` at the level above. Communities are numbered from 0 by decreasing size at every level, so `community` 0 is the largest. `internal_edges`: edges with both ends inside; `edges_out`: edges with one end inside; `share`: the fraction of all nodes. Counts are exact at every level. | #62 |
| `c.nodes(level=1)` | `node`, `community` | The community of every node. Handing `c` itself to Polars or DuckDB gives this table. | #57, #62, #83 |
| `c.of(nodes)` | `level`, `community`, `nodes` | Which community a node is in at every level, and how large that community is. For several nodes, a leading `node` column. | #62 |
| `c[i]` | a `Community` | Community `i` of the partition, usable wherever a node set is expected: `g.subgraph(c[0])`, `avoid=c[3]`, `m.highlight(c[0])`. | #62, #87 |

It prints as approved in `api.md` § 3 (stories 1 and 4). The lines, in order: what and
how (`Leiden, resolution 1.0, deterministic` — or `fast`, and `precision low` when that
applies); the number of communities, the modularity, the number of disconnected
communities (always 0 for Leiden; it is printed because it is the guarantee); the size
of each level; the largest, the median and the singletons; a `note` line when direction
was ignored on a directed graph; the time taken. The notebook card is cell 4 of
`notebook.html`.

### 3.6 `Hierarchy`

The tree a map is drawn from (`docs/DESIGN.md` D16). It is not the same thing as the
levels of `Communities`: it is shaped for reading — no region has too many children —
and so it contains three kinds of region, which the map draws differently and never
confuses (`SPEC.md`, "The map"):

| `kind` | What it is | On the map |
|---|---|---|
| `community` | A community found by the algorithm. | solid outline |
| `subdivision` | A split made only so the map stays readable. | dashed outline |
| `aggregate` | Many small communities shown as one. | hatched |

```python
class Hierarchy:
    levels: int
    def table(self, level: int | None = None) -> Table: ...
    def of(self, nodes: Nodes) -> Table: ...
    def children(self, region: int) -> Table: ...
```

| Member | Returns | Notes | Implemented by |
|---|---|---|---|
| `levels` | `int` | The number of zoom levels; level 1 is the top. | #63 |
| `table(level=None)` | one row per region | All levels, or one. `region` numbers are unique within the hierarchy; `parent` is null at level 1; `name` is the region's label on the map, or null. Counts are exact. | #62, #63 |
| `of(nodes)` | `level`, `region`, `kind`, `name`, `nodes` | The regions containing a node, from the top down. | #62 |
| `children(region)` | the rows of `table()` whose parent is `region` | | #62 |

### 3.7 `Path`

```python
class Path:
    found: bool
    hops: int | None                  # None when not found
    truncated: bool                   # the search stopped at limit or timeout
    meta: Mapping[str, object]

    def nodes(self) -> Table: ...     # step, node
    def edges(self) -> Table: ...     # step, source, target, edges, weight,
                                      # first, last
    def alternatives(self, limit: int = 10) -> list[Path]: ...
    def on_map(self) -> Map: ...
```

| Member | Notes | Implemented by |
|---|---|---|
| `found`, `hops` | No path within the constraints is a result, not an error: `found` is `False`. If the search was cut short as well, a path may still exist, and the text says so. | #91 |
| `nodes()`, `edges()` | The path as tables. `edges()` has one row per step: the pair, how many edges join it, their summed weight, the first and last time. | #91 |
| `alternatives(limit)` | The other shortest paths of the same length, at most `limit` †. The only place a Python list is returned; it is short by construction. | #91 |
| `on_map()` | Draws the path on the graph's map, dimming the rest. | #76 |
| `meta` | `avoided` (how many nodes were excluded), `alternatives` (how many other shortest paths exist, counted up to a bound), `visited`, `took_seconds`. | #91 |

Text form, approved in `api.md` § 4 (story 3): `Path`, the hops, the time; one line per
step with the weight and the time on the arrow; a last line with what was avoided and
how many alternatives exist. A step whose pair has several edges shows their sum and
their number (`value 37.200 · 4 edges`). The weight is labelled with its column's name.
When nothing is found:

```text
>>> g.path("0x4f3a…91bc", "0x9e12…07aa", max_hops=2)
Path · none within 2 hops · searched in 0.4 ms
  from 0x4f3a…91bc to 0x9e12…07aa
  to look further: g.path("0x4f3a…91bc", "0x9e12…07aa", max_hops=5)
```

### 3.8 `Map` and `Selection`

```python
class Map:
    file: pathlib.Path                # the .egmap
    levels: int                       # zoom levels
    hierarchy: Hierarchy
    selection: Selection

    def export(self, path: str | os.PathLike, *,
               reveal: Literal["full", "summary"] = "full",
               **run: Unpack[Run]) -> pathlib.Path: ...
    def highlight(self, nodes: Nodes | None) -> None: ...

class Selection:
    kind: Literal["none", "node", "region"]
    def nodes(self) -> Table: ...
```

| Member | Notes | Implemented by |
|---|---|---|
| `g.map()` | § 3.2. The map file is `NAME.egmap` beside the graph (a graph in memory keeps it in scratch until exported). Calling it again returns the same map without rebuilding. | #63, #67, #69 |
| displaying `m` | In a notebook: the interactive map, the same viewer as `everygraph open`, with its caption strip. At a prompt: the text form of story 5. | #76, #71, #78 |
| `m.export(path, reveal=)` | Writes a map file for sharing, with `viewer.html` beside it: one static file to host anywhere. `reveal="full"`: every node identifier and the links shown. `reveal="summary"`: regions and counts only, no individual identifiers (`choices.md` C3). A map never contains more than it shows. | #75 |
| `m.highlight(nodes)` | Marks these nodes on the displayed map; `None` clears. A table's other columns are not used in 0.1. | #76 |
| `m.selection` | What is selected on the displayed map right now. `kind` is `"none"` before anything is clicked. | #76 |
| `m.selection.nodes()` | The selected nodes as a table (story 5): the node itself, or every node of the selected region, largest `total` first. Nothing selected gives a table with the same columns and no rows, whose footer says how to select — not an error. A `Selection` is a node set (§ 2.2). | #76 |
| `m.hierarchy` | § 3.6. | #63 |
| `m.file`, `m.levels` | | #69 |

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

### 3.10 Exceptions

```python
class EverygraphError(Exception):
    message: str
    ways_forward: list[tuple[str, str]]      # (code to copy, note)
    changed: bool
    def to_dict(self) -> dict: ...

class InsufficientBudget(EverygraphError): ...
class InsufficientDisk(EverygraphError): ...
class InvalidInput(EverygraphError): ...
class InvalidExpression(EverygraphError, ValueError): ...
class UnknownNode(EverygraphError, KeyError): ...
class Unsupported(EverygraphError): ...
class CorruptFile(EverygraphError): ...
```

When each is raised, their attributes and their exit statuses are in § 2.7; the message
pattern is in `docs/ux-contract.md` § 6.1. In a notebook an exception is shown as the
error card, explanation first and traceback folded (#76).

---
## 4. The Rust embedding API

For the developer who embeds a graph or a map in an application (`docs/DESIGN.md` § 2).
The Python package and the command are thin layers over these crates, so this section
mirrors § 3 name for name: free functions instead of methods, a struct of parameters
instead of keywords, `Result` instead of exceptions.

These are signatures, not code: the workspace is being created by #2, and the crates'
internals belong to their own issues. Lifetimes, generic bounds and the exact module
paths are indicative; the names, the shape of the parameters and results, and which
crate owns what are the specification. The Rust API is unstable in 0.1 (§ 2.10).

```rust
// ---- everygraph-core: shared types (#2, #31, #44, #45) -------------------

pub struct NodeId(pub u32);        // dense: 0..num_nodes (D3)
pub type EdgeOffset = u64;

pub enum Direction { Out, In, Both }
pub enum Precision { Full, Low }
pub enum Weight<'a> { Declared, Column(&'a str), None }   // True, "name", False

/// What every planned operation takes: the `**run` of the Python API.
/// `Run::default()` gives the documented defaults and discards events.
pub struct Run<'a> {
    pub memory: Option<u64>,             // bytes; None: half of free memory
    pub threads: Option<NonZeroUsize>,   // None: the planner chooses
    pub temp_dir: Option<&'a Path>,
    pub events: &'a dyn Events,          // plan, progress, warnings
    pub cancel: &'a Cancel,              // checked at chunk boundaries
}

pub trait Events: Sync {
    fn event(&self, event: &Event);      // never called concurrently
}
pub enum Event {                         // ux-contract § 4.6, one to one
    Plan(Plan), Phase(Phase), Warning(Warning),
    Result(Outcome), Stopped(Stopped), Error(ErrorDoc),
}
pub struct Cancel { /* an atomic flag */ }
impl Cancel {
    pub fn new() -> Cancel;
    pub fn cancel(&self);                // what Ctrl-C calls
    pub fn is_cancelled(&self) -> bool;
}

pub struct Plan { /* the fields of ux-contract § 3.2 */ }
impl Plan {
    pub fn fits(&self) -> bool;
    pub fn refusal(&self) -> Option<&Refusal>;
    pub fn to_json(&self) -> String;     // the schema of ux-contract § 3.6
}
impl fmt::Display for Plan { /* the text form */ }

pub enum Error {                         // § 2.7, one to one
    InsufficientBudget(Refusal),
    InsufficientDisk(Refusal),
    InvalidInput(InvalidInput),
    InvalidExpression { expression: String, position: usize, why: String },
    UnknownNode { node: String },
    Unsupported { operation: String, reason: String },
    CorruptFile { path: PathBuf, section: Option<String> },
    Stopped(Stopped),                    // KeyboardInterrupt in Python
    Io(io::Error),                       // passed through, as in Python
}
impl Error {
    pub fn ways_forward(&self) -> &[Way];
    pub fn changed(&self) -> bool;
    pub fn to_json(&self) -> String;     // the schema of ux-contract § 6.1
}
pub type Result<T> = core::result::Result<T, Error>;

/// Implemented by `Graph` and by `View`: anything an algorithm can run on.
pub trait GraphRef {
    fn num_nodes(&self) -> u64;
    fn num_edges(&self) -> u64;
    fn is_directed(&self) -> bool;
    fn columns(&self) -> &[ColumnInfo];
    fn resolve(&self, ids: &Ids) -> Vec<Option<NodeId>>;   // in bulk (#34)
    fn id_of(&self, node: NodeId) -> Id<'_>;
}

/// A result. Row i is node i unless the table has its own `node` column.
/// Columns are Arrow-compatible buffers; `everygraph-py` exports them
/// through the Arrow C interface without copying (#83).
pub struct Table { /* columns, meta */ }
impl Table {
    pub fn columns(&self) -> &[Column];
    pub fn num_rows(&self) -> u64;
    pub fn column<T: Value>(&self, name: &str) -> Option<ColumnRef<'_, T>>;
    pub fn meta(&self) -> &Meta;         // parameters, plan, took, cached …
    pub fn truncated(&self) -> Option<&Truncated>;
    pub fn approximate(&self) -> Option<&Approximate>;
    pub fn top(&self, n: usize, by: Option<&str>) -> Table;
    pub fn head(&self, n: usize) -> Table;
}

// ---- everygraph-format: the graph file (#28, #39, #41) --------------------

pub struct Graph { /* a .eg file, or a graph in memory */ }
impl Graph {
    pub fn open(path: &Path) -> Result<Graph>;
    pub fn file(&self) -> Option<&Path>;
    pub fn info(&self) -> Info;          // what `everygraph info` prints
    pub fn nodes(&self) -> Table;
    pub fn edges(&self) -> Table;
}
impl GraphRef for Graph { /* … */ }
pub fn save(g: &impl GraphRef, path: &Path, run: &Run) -> Result<PathBuf>;
pub fn verify(path: &Path, deep: bool, run: &Run) -> Result<VerifyReport>;

// ---- everygraph-ingest (#32–#36, #40, #84) --------------------------------

pub enum Input<'a> { File(&'a Path), Table(&'a dyn EdgeTable) }
pub struct Read<'a> {
    pub source: Option<&'a str>,
    pub target: Option<&'a str>,
    pub weight: Option<&'a str>,
    pub time: Option<&'a str>,
    pub directed: Option<bool>,
    pub out: Option<&'a Path>,
}
pub fn read(input: Input, read: &Read, run: &Run) -> Result<Graph>;
pub fn plan_read(input: Input, read: &Read, run: &Run) -> Result<Plan>;
pub fn from_edges(source: &Ids, target: &Ids, directed: bool, run: &Run)
    -> Result<Graph>;

// ---- everygraph-kernels (#44–#51) -----------------------------------------

pub struct PageRank<'a> {
    pub damping: f64,                    // 0.85
    pub tol: f64,                        // 1e-6 †
    pub max_iter: u32,                   // 100 †
    pub weight: Weight<'a>,
    pub personalize: Option<&'a [NodeId]>,
    pub precision: Precision,
}
pub fn pagerank(g: &impl GraphRef, p: &PageRank, run: &Run) -> Result<Table>;
pub fn components(g: &impl GraphRef, strong: bool, run: &Run) -> Result<Table>;
pub fn kcore(g: &impl GraphRef, run: &Run) -> Result<Table>;
pub fn triangles(g: &impl GraphRef, run: &Run) -> Result<Table>;
pub fn degrees(g: &impl GraphRef, nodes: Option<&[NodeId]>, weight: Weight)
    -> Result<Table>;
pub fn bfs(g: &impl GraphRef, source: NodeId, direction: Direction,
           max_hops: Option<u32>, run: &Run) -> Result<Table>;
pub fn top(g: &impl GraphRef, by: &str, n: usize, run: &Run) -> Result<Table>;

/// `g.explain(...)` and `dry_run=True`. The planner's crate is #45's choice;
/// it is shown here because the kernels declare their costs to it.
pub enum Operation<'a> {
    PageRank(PageRank<'a>), Components { strong: bool }, KCore, Triangles,
    Bfs { source: NodeId, direction: Direction, max_hops: Option<u32> },
    Communities(Communities<'a>), Map(MapParams<'a>),
}
pub fn explain(g: &impl GraphRef, op: &Operation, run: &Run) -> Result<Plan>;

// ---- everygraph-community (#57, #58, #60, #62, #63, #65) -------------------

pub enum Method { Leiden, LabelPropagation }
pub enum Objective { Modularity, Cpm }
pub struct Communities<'a> {
    pub method: Method,
    pub resolution: f64,                 // 1.0
    pub objective: Objective,
    pub iterations: u32,                 // 1
    pub weight: Weight<'a>,
    pub directed: bool,                  // false: direction is ignored
    pub deterministic: bool,             // true
    pub seed: u64,                       // 0
    pub precision: Precision,
}
pub fn communities(g: &impl GraphRef, p: &Communities, run: &Run)
    -> Result<Partition>;

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
    pub fn children(&self, region: u32) -> Table;
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
pub fn has_edge(g: &impl GraphRef, source: NodeId, target: NodeId) -> bool;
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

pub enum Reveal { Full, Summary }
pub struct MapParams<'a> {
    pub communities: Option<&'a Partition>,
    pub deterministic: bool,
    pub seed: u64,
}
pub fn map(g: &impl GraphRef, p: &MapParams, run: &Run) -> Result<MapFile>;
pub struct MapFile { /* a .egmap */ }
impl MapFile {
    pub fn file(&self) -> &Path;
    pub fn levels(&self) -> u32;
    pub fn hierarchy(&self) -> &Hierarchy;
    pub fn export(&self, path: &Path, reveal: Reveal, run: &Run)
        -> Result<PathBuf>;
}
```

How the two APIs differ, on purpose:

| Python | Rust | Why |
|---|---|---|
| Identifiers in and out | `NodeId` in and out; `resolve` and `id_of` convert in bulk | The embedding developer usually has dense numbers already; the conversion is one explicit call instead of one per call. |
| Exceptions | `Result<T, Error>` | The same seven kinds, the same fields. `Stopped` is an error value here; Python turns it into `KeyboardInterrupt`. |
| `progress=` | `Run::events` | The same events. The command and the Python package render them with the one renderer of `docs/ux-contract.md` § 8. |
| `dry_run=True` | `explain`, `plan_read` | A function that returns a `Plan` is clearer in Rust than a flag that changes the return type. |
| `Selection`, `highlight`, the widget | — | They belong to the viewer (#71, #76), not to the engine. |
| `to_networkx` and the other converters | — | Python-side by nature. |

Constraints the signatures respect (`docs/FINDINGS.md` § G): nothing here takes a file
handle or assumes threads — `Graph::open` and `read` are in the crates that may touch
the file system, while the kernels see only `GraphRef`; every long function takes a
`Cancel` and can be run again; no function returns a structure that needs a single
allocation proportional to the graph.

---

## 5. The command line

The command is `everygraph`; no short alias is installed (`cli.md` Choice 1). Its shape
is verb, then thing (`cli.md` Choice 5). Everything it prints, and where, is specified
in `docs/ux-contract.md`; this section fixes what can be typed.

### 5.1 Commands

```text
everygraph ingest FILE   [--source NAME] [--target NAME] [--weight NAME]
                         [--time NAME] [--directed | --undirected] [--out FILE]
everygraph info GRAPH
everygraph run NAME GRAPH    [options of NAME] [VIEW] [--top N] [--out FILE]
everygraph query NAME GRAPH NODE [NODE]
                             [options of NAME] [VIEW] [--top N] [--out FILE]
everygraph map GRAPH     [VIEW] [--out FILE] [--reveal full|summary]
everygraph open GRAPH
everygraph export GRAPH  [VIEW] --out FILE
everygraph verify FILE   [--deep]
everygraph doctor

VIEW := [--window START END] [--edges EXPRESSION] [--nodes EXPRESSION]
```

| Command | Does | Python | Implemented by |
|---|---|---|---|
| `ingest FILE` | Reads a file of edges and builds `FILE.eg` beside it (the input's name with its extension replaced). Always writes a file. | `eg.read(file, …)` | #98; #32–#36, #40 |
| `info GRAPH` | What the file holds and what was computed so far. | `g.info()` | #98; #39 |
| `run NAME GRAPH` | A whole-graph algorithm. `NAME`: `pagerank`, `components`, `kcore`, `triangles`, `degrees`, `bfs` (takes one `NODE`), `communities`. Its options are the method's parameters (§ 3.2) with dashes. | `g.NAME(…)` | #98; #47–#51, #57 |
| `query NAME GRAPH NODE …` | A question about particular nodes. `NAME`: `neighbors`, `khop`, `path` (takes two nodes). | `g.NAME(…)` | #98; #90, #91 |
| `map GRAPH` | Builds whatever is missing and writes `GRAPH.egmap`. With `--out`, writes a map for sharing there instead, with `viewer.html`. | `g.map()`, `m.export(…)` | #98; #63, #67, #69, #75 |
| `open GRAPH` | Serves the map to a browser on this computer only (`127.0.0.1`) until `Ctrl-C`. | displaying `m` | #74 |
| `export GRAPH --out FILE` | Writes the graph, or a view of it, in the format of `FILE`'s extension. | `g.save(path)` | #85 |
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
| `--top N` | `.top(n)` | `run`, `query` |
| `--deterministic` / `--fast` | `deterministic=` | `run communities`, `map` |
| `--seed N` | `seed=` | `run communities`, `map` |
| `--precision full\|low` | `precision=` | `run` |
| `--weight NAME` / `--no-weight` | `weight=` | `run`; on `ingest`, `--weight NAME` names the weight column |
| `--limit N`, `--timeout SECONDS` | `limit=`, `timeout=` | `query` |
| `--window START END` | `g.window(start, end)` | `run`, `query`, `map`, `export` |
| `--edges EXPRESSION`, `--nodes EXPRESSION` | `g.filter(edges=…, nodes=…)` | `run`, `query`, `map`, `export` |
| `-h`, `--help`; `--version` | — | everywhere |

Options particular to one name:

| Command | Options | Python |
|---|---|---|
| `ingest` | `--source NAME`, `--target NAME`, `--time NAME`, `--directed`, `--undirected` | the parameters of `eg.read` |
| `run pagerank` | `--damping X`, `--tol X`, `--max-iter N`, `--personalize FILE` | `g.pagerank(…)` |
| `run components` | `--strong` | `strong=True` |
| `run bfs` | `--direction out\|in\|both`, `--max-hops N` | `g.bfs(…)` |
| `run communities` | `--method NAME`, `--resolution X`, `--objective NAME`, `--iterations N`, `--directed` | `g.communities(…)` |
| `query neighbors` | `--direction out\|in\|both` | `g.neighbors(…)` |
| `query khop` | `--k N`, `--direction out\|in\|both` | `g.khop(…)` |
| `query path` | `--max-hops N`, `--avoid FILE`, `--avoid-hubs N`, `--weight NAME` | `g.path(…)`; `--avoid-hubs N` is `avoid=g.top("degree", N)` |
| `map` | `--reveal full\|summary` (with `--out`) | `m.export(…, reveal=…)` |
| `verify` | `--deep` | — |

A `FILE` given to `--avoid` or `--personalize` is a text file with one identifier per
line, or any table file with a column named `node`.

Python only in 0.1, because they need a session: `eg.from_edges`, `g.subgraph` and the
node sets it takes (`c[i]`, a selection, a `khop` result), `c.of`, `m.highlight`,
`m.selection`, `p.alternatives`, the converters to NetworkX, igraph and SciPy, and
`progress=`.

Not provided, by decision: a confirmation prompt and `--yes` (nothing ever asks); a
quiet or verbose switch (`cli.md` Choice 2); a short alias.

### 5.3 What a result looks like

One result, three forms (`cli.md` Choice 6), chosen as in `docs/ux-contract.md` § 2.2:

- **On a terminal**: a table aligned with spaces, numbers with separators, identifiers
  shortened only if the table would pass 80 columns, and a dim footer (story 2). Without
  `--top`, the first ten rows and `10 of 212,418,907 rows shown`.
- **Piped**: every row, tab-separated, with a header row; plain numbers; full
  identifiers; no footer.
- **`--json`**: one document.

```json
{
  "graph": "transfers.eg",
  "result": "pagerank",
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
  "cached": true,
  "truncated": null,
  "approximate": null
}
```

`run communities` prints the summary of § 3.5; with `--top N` the table of the `N`
largest communities; with `--out` or a pipe, the membership table (`node`,
`community`). `query path` prints the two or three lines of story 3. `info` and `doctor`
print labels and values. With `--out FILE` the table goes to the file and a result line
says what was written.

`everygraph --help`, as typeset in `terminal.html`, with the lines this specification
adds (`export`, `verify`, `--threads`, `--temp-dir`, and the longer lists after `run`
and `query`):

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
  -h, --help            this text; after a command, the help for that command
  --version             print the version

Start here
  everygraph ingest FILE
```

---

## 6. Every public name, its issue and its story

"Story" is the story of § 1 that exercises the name; a dash means no story does, and the
name is covered by the tests of its own issue only. ‡ and † as in § 3.

**Python: functions and `Graph`**

| Name | Implemented by | Story |
|---|---|---|
| `eg.read` | #81; #32, #33, #34, #35, #36, #40 | 1, 2, 9 |
| `eg.read` with a table as `data` | #83, #95 | — |
| `eg.open` | #81; #28, #41 | 3 |
| `eg.from_edges` ‡ | #84 | 10 |
| `eg.from_scipy` ‡ | #84 | — |
| `g.num_nodes`, `g.num_edges`, `g.directed`, `g.file` | #81 | 4 |
| `g.columns` | #37 | — |
| `g.info()` | #39, #98 | — (§ 3.2) |
| `g.nodes()`, `g.edges()` | #81, #83 | — |
| `g.save()` ‡ | #84, #85 | 7, 10 |
| `g.pagerank()` | #47 | 2, 5, 6 |
| `g.components()` | #48 | — |
| `g.kcore()` | #50 | — |
| `g.triangles()` | #51 | 10 |
| `g.degrees()` | #50, #90 | — |
| `g.bfs()` | #49 | — |
| `g.top()` | #81; #50, #47 | 3 |
| `g.communities()` | #57, #58, #60, #62, #65 | 1, 4, 6, 7, 8 |
| `g.map()` | #63, #67, #69 | 5 |
| `g.explain()` | #45 | 8 |
| `g.neighbors()` | #90, #92 | 3 |
| `g.khop()` | #90, #92 | 3, 7 |
| `g.path()` | #91, #38, #92 | 3 |
| `g.has_edge()`, `g.common_neighbors()`, `g.edges_between()` ‡ | #90 | — |
| `g.filter()` | #93, #37 | 4 |
| `g.window()` | #94, #93 | 4 |
| `g.as_of()` ‡ | #94 | — |
| `g.subgraph()` | #87, #93, #62 | 7 |
| `g.to_networkx()` ‡ | #85, #87 | 7 |
| `g.to_igraph()`, `g.to_scipy()` ‡ | #85 | — |

**Python: the other objects**

| Name | Implemented by | Story |
|---|---|---|
| `View`, `view & view`, `view.base`, `view.definition` | #93 | 4 |
| `Table`: `columns`, `num_rows`, `len()` | #81 | — |
| `Table.truncated` | #90 | 3 |
| `Table.approximate` | #52 | — |
| `Table.meta` | #45, #39 | — |
| `Table.head()` | #81 | 1 |
| `Table.top()` | #47, #81 | 2, 5, 6 |
| `Table.sizes()` | #48, #50 | — |
| `Table.on_map()` | #76 | — |
| `Table.write()` | #83, #85 | 1 |
| `Table.to_arrow()`, `Table.to_pandas()`, the Arrow interface | #83 | 6 |
| `Communities`: `len()`, `modularity`, `levels` | #57, #62 | 1 |
| `c.table()` | #62 | 1 |
| `c.nodes()` | #57, #62 | 1, 6 |
| `c.of()` | #62 | — |
| `c[i]`, `Community` | #62, #87 | 7 |
| `Hierarchy`: `levels`, `table()`, `of()`, `children()` | #62, #63 | — |
| `Path`: `found`, `hops`, `truncated`, `meta` | #91 | 3 |
| `p.nodes()`, `p.edges()` | #91 | — |
| `p.alternatives()` | #91 | 3 |
| `p.on_map()` | #76 | — |
| `Map`, displaying it | #76, #71, #78 | 5 |
| `m.export()` | #75 | 5 |
| `m.highlight()` | #76 | 5 |
| `m.selection`, `Selection.kind`, `Selection.nodes()` | #76 | 5 |
| `m.hierarchy`, `m.file`, `m.levels` | #63, #69 | — |
| `Plan`: `fits`, `operation`, `estimate`, `refusal`, `to_dict()` | #45, #40 | 8 |
| `EverygraphError` and its attributes | #81 | 8 |
| `InsufficientBudget` | #31, #45 | 8 |
| `InsufficientDisk` | #40 | — |
| `InvalidInput` | #32, #33 | — |
| `InvalidExpression` | #93 | — |
| `UnknownNode` | #34, #90 | — |
| `Unsupported` | #81; #85, #94 | 7 |
| `CorruptFile` | #28, #41 | — |

**Parameters and environment**

| Name | Implemented by | Story |
|---|---|---|
| `memory=`, `--memory`, `EVERYGRAPH_MEMORY` | #31, #45 | 8 |
| `threads=`, `--threads`, `EVERYGRAPH_THREADS` | #45 | — |
| `temp_dir=`, `--temp-dir`, `EVERYGRAPH_TEMP_DIR` | #40 | — |
| `deterministic=`, `--deterministic`, `--fast` | #57, #54 | 1 (default) |
| `seed=`, `--seed` | #57 | — |
| `precision=`, `--precision` | #45, #47 | 8 |
| `weight=`, `--weight`, `--no-weight` | #36, #47, #58 | 2 (`eg.read`) |
| `limit=`, `--limit` | #90, #91 | 3, 7 |
| `timeout=`, `--timeout` | #90, #91 | — |
| `dry_run=`, `--dry-run` | #40, #45 | 8 |
| `progress=` | #81 | — |
| `direction=`, `max_hops=`, `avoid=`, `k=` | #90, #91 | 3 |
| `source=`, `target=`, `time=`, `directed=` of `eg.read`; `--source`, `--target`, `--time`, `--directed`, `--undirected` | #32, #33, #36 | 2 |
| `damping=`, `tol=`, `max_iter=`, `personalize=` | #47 | — |
| `strong=`, `--strong` | #48 | — |
| `method=`, `resolution=`, `objective=`, `iterations=`, `directed=` of `communities` | #57, #58, #65 | — |
| `events=`, `max_edges=` | #85, #87 | 7 |
| `ids=` | #81, #83 | — |
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
| `--json` | #98 | 8 |
| `--top` | #98 | 2 |
| `--window`, `--edges`, `--nodes` | #98; #93, #94 | 4, 7 |
| `--avoid`, `--avoid-hubs` | #98; #91 | 3 |

The Rust names of § 4 map crate by crate: `everygraph-core` #2, #31, #44, #45;
`everygraph-format` #28, #39, #41; `everygraph-ingest` #32–#36, #40, #84;
`everygraph-kernels` #44–#51; `everygraph-community` #57, #58, #60, #62, #63, #65;
`everygraph-query` #90–#94; `everygraph-map` #63, #67–#69, #75. The stories exercise
them through Python (#81) and the command (#98).

Every story maps to issues in M2–M7, as the acceptance of #12 requires:

| Story | M2 | M3 | M4 | M5 | M6 | M7 |
|---|---|---|---|---|---|---|
| 1 A file to communities | #32, #34, #35, #36 | | #57, #58, #62 | | #81, #83, #84 | |
| 2 Larger than memory | #33, #35, #39, #40 | #45, #46, #47, #53 | | | #81 | |
| 3 Neighbours and a path | #38 | #50 | | | #81 | #90, #91, #92 |
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
| A low-memory variant of community detection (`precision="low"`: "needs 3.4 GB; slightly lower quality"). #57 keeps 64-bit weights on purpose; #45 mentions lower-memory variants only in general; only PageRank (#47) has one planned. | `api.md` § 6, `cli.md` § 2 | `precision` is specified for operations that have the variant; the refusal lists it only where it exists. Whether communities get one is a decision for #57/#60. |
| A unit on path weights (`12.400 ETH`). Nothing in the API declares a unit. | `api.md` § 4 | The weight is labelled with its column's name (§ 9, item 4). |
| Automatic names for regions ("around 0x28c6…1d60 and 2 others", or from a `label` column). #62 computes per-community summaries "for map labels"; no issue owns the naming rule. | `choices.md` B3 | `Hierarchy.table()` has a `name` column; who fills it is open. Suggested: #63. |
| The user's own nouns in summaries ("addresses", "transfers"). No rule says how they are derived. | `design/preview/README.md` N3, `SPEC.md` "Words" | The noun for nodes comes from the kind of identifier (`hex addresses` → "addresses"); no noun is derived for edges in text. A fuller rule belongs to #121. |
| Node columns. `g.filter(nodes=…)`, colouring a map by a label and B3 all assume nodes can have columns, but every preview reads edges only. | `api.md` § 5 | `nodes=` works on `in`, `out`, `total` and on node columns if a format supplies them (GraphML, GEXF). A way to attach a node table (`eg.read(…, nodes=table)`) needs a preview. |
| `viewer.html` beside a shared map; #75 writes "a single `index.html` embedding the viewer". | `cli.md` § 1 | `viewer.html`, as approved. #75's text should follow. |

**Promised by an issue, not exposed here (or exposed differently)**

| What | Issue | Here |
|---|---|---|
| `Graph.view(filter)`; `khop(…, budget=…)`; an option `memory_budget`; "requests carry budgets" | #81, #90, #74, #87 | The approved names are `g.filter()`, `limit=`, `memory=`. The issue texts predate the decision that "budget" means memory only. |
| `--yes`; "quiet structured logs" when not on a terminal | #98 | Nothing asks, so no `--yes`. The approved piped form is a plain log; `--json` gives one document (`docs/ux-contract.md` § 9). |
| "User overrides for every choice" of the planner | #45 | Exposed: `memory`, `threads`, `precision`, `deterministic`. Forcing a regime or a decode tier has no public name in 0.1; if #45 needs one for testing it stays out of the documented API. |
| Weighted single-source distances; multi-source traversal | #49 | `bfs` (hops) only. They remain internal building blocks until a name is previewed. |
| Betweenness, closeness, harmonic and eigenvector centrality | #52 (P2) | Not in § 3. By the naming rule they would be `g.betweenness()`, `g.closeness()`, `g.harmonic()`, `g.eigenvector()`, the first returning an approximate table (§ 2.5). To be confirmed when #52 starts. |
| The giant component and the k-core "as a reusable view" | #48, #50 | Reachable today as `g.subgraph(t)` with a table filtered in Polars. Filters over computed columns (`nodes="component == 0"`) would be the short form; not specified. |
| Degree histogram and quantiles | #50 | The `degrees()` table; statistics are left to the table library. |
| Comparing partitions (NMI, ARI), per-community metrics, stability reports | #64 (P2) | Not in § 3; no previewed names. |
| Activity series and counts per time bucket | #94 | Not in § 3; `g.window()` and `g.as_of()` only. |
| "Results … back as a node property" | #83 | No call attaches a table to a graph (it is immutable). The join happens in the table library (story 6); node sets come back through `subgraph`, `highlight`, `avoid`. |
| `eg.from_dataframe` | #84 | Folded into `eg.read(table)`. |
| Batch forms "returning Arrow list columns" | #90 | A flat table with a leading `node` column: the same shape as every other result, and it joins directly. |
| All shortest paths, k shortest paths, a batch of pairs | #91 | `p.alternatives()` covers the first. The other two have no name here. |
| `g.subgraph(…)` returning "a small `Graph` (in memory)" | #87 | It returns a `View`, as #62 and the preview have it; the planner materializes it when small. |
| `everygraph map export`; `everygraph compact` | #75; #39, #28 | `everygraph map GRAPH --out FILE` for the first. `compact` is not in #98's list of commands nor in the approved help; it needs adding there or dropping. |
| NumPy views of numeric columns | #83 | Through Arrow (`pl.from_arrow(t)["pagerank"].to_numpy()` does not copy); no direct `t["pagerank"]` in 0.1. |
| A DuckDB extension with table functions | #95 | Outside the Python API; #95 scopes it as a prototype. |

**Between the design documents**

| What | Here |
|---|---|
| Threads by default: `choices.md` A5 says "all physical cores"; `DESIGN.md` D13 lets compute-bound kernels use every hardware thread. | § 2.4: physical cores is the default and the bound the user sees; the planner may add the sibling threads only when `threads` is not given. |
| Results "stored in transfers.eg" (`cli.md`) against #39, which has not decided between the file and a companion directory. | § 2.8 says "inside the graph file" as approved and notes #39 decides the mechanism; what the user sees is the same. |
| `duckdb.sql("select … from pr")` on an Everygraph table (`api.md` Choice 4) relies on DuckDB accepting any Arrow-stream object by variable name. | Kept as approved; #95 must test it, and fall back to documenting `duckdb.from_arrow(pr)` if it does not hold. |
| `docs/DESIGN.md` § 3 names no crate that a Rust user would depend on to get everything. | § 4 writes the functions under the crates that own them. A facade crate `everygraph` would be a small addition to #2. |
| `docs/spec/graph-model.md` (#36) will define what each algorithm does on directed, weighted and repeated edges. | § 2.4 (`weight`) and § 3.3 (which nodes a view keeps) state the user-visible defaults; #36 must confirm them or change this document. |

---

## 8. Non-goals for 0.1

Decided, and deliberately absent from everything above.

| Not in 0.1 | Why, and where it went |
|---|---|
| **Changing a graph**: adding or removing nodes or edges, editing a column, appending new data to a file. | A graph is immutable and file-backed (#81, `docs/DESIGN.md` § 2: not a transactional database). A changed input is read again. Appends are designed in #94 and built after 0.1 (#111). |
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
of reasoning; each can be overturned by the owner without touching the rest. Nothing
here changes an answer in `design/DECISIONS.md`.

**A. Choices**

1. **`explain` is a method and `dry_run=True` is the same thing.**
   `g.explain("communities", …)` returns a `Plan`; so does any planned call with
   `dry_run=True`, and `eg.read(…, dry_run=True)` is the form for reading, where there is
   no graph yet. *`notebook.html` shows the method, `api.md` the flag, #45 and #81 the
   word "explain": one object, two spellings, no third.*
2. **`m.selection.nodes()` returns a table** — `node`, `community`, `in`, `out`, `total`
   — and an empty table, not an error, when nothing is selected. *As drawn in
   `notebook.html` (cell 11 and "an empty result"); a table is what every other call
   returns.*
3. **`precision` takes `"full"` (default) or `"low"`.** *"low" is the approved word;
   "full" says the default is not an upgrade. Asking for it where no variant exists is an
   error rather than a silent no-op.*
4. **What prints.** Tables have no `shape:` line and always a footer (rows shown of
   total, order); more than ten rows show five, `…` and the last; numbers are
   right-aligned, `share` included. A graph prints the head of `everygraph info`. A path
   labels its weight with the column's name instead of `ETH`. *`api.md` showed three
   table layouts; the typeset cards all have the footer. The engine knows a column's
   name, never its unit.*
5. **Numbering.** Communities and components are numbered from 0 by decreasing size, at
   every level; levels are numbered from 1 at the coarsest, as on the map ("level 1
   of 6"), so level 1 of `Communities` is the partition. *`c[0]` and "community 0" in the
   approved tables are the largest; one direction for levels everywhere.*
6. **`weight=True` by default**: use the weight column declared at `eg.read` if there is
   one, otherwise count the edges between a pair. *`choices.md` A1 says a pair is
   analysed once "with a weight (count or summed amount)"; a column the user declared is
   not the implicit-attribute trap FINDINGS § F2 warns about. #36 must confirm.*
7. **Views.** `g.subgraph()` returns a `View`. Filtering edges or taking a window keeps
   only the nodes that still have an edge; filtering nodes keeps the edges between them.
   A window includes both ends, and a date as its end includes that day; times are UTC.
   *Only this reading makes `g.window("2024-03-01", "2024-03-31")` mean March, as the
   preview intends; and communities of a window should not report two hundred million
   isolated nodes.*
8. **The map file.** `g.map()` writes `NAME.egmap` beside the graph, exactly as
   `everygraph map` does; `m.export(path, reveal="full" | "summary")` writes the copy to
   share. *`choices.md` C3 asks for the choice at export and names no parameter;
   "reveal" is its own word ("what a shared map reveals").*
9. **Command-line spellings of what the previews show only in Python**: an option is the
   parameter's name with dashes (`--max-hops`, `--k`, `--limit`, `--direction`); views
   are `--window START END`, `--edges`, `--nodes`; `avoid=g.top("degree", N)` is
   `--avoid-hubs N`, and `--avoid FILE` takes a list. *One rule instead of a table to
   memorize; "avoid hubs" is the viewer's own label.*
10. **Writing things out.** `t.write(path)` is the Python form of `--out`, `g.save(path)`
    of `everygraph export`, both choosing the format from the extension. Defaults can
    also come from `EVERYGRAPH_MEMORY`, `EVERYGRAPH_THREADS`, `EVERYGRAPH_TEMP_DIR`.
    *The previews show `--out` and `m.export` but no Python call for a table or a
    graph; scripts on shared machines need the environment.*
11. **Errors.** Seven classes under `EverygraphError` (§ 2.7); argument mistakes and
    operating-system errors stay Python's own; reaching a limit is never an error.
    *The previews name one class and #81 two; the rest follow from the failures the
    previews and issues describe.*
12. **Shapes of two results.** `g.components(strong=True)` for strongly connected
    components. `g.neighbors()` returns one row per distinct neighbour with `edges`, the
    summed weight, `first` and `last`; several nodes give one flat table. *Short names
    (Choice 13); repeated edges are events, so a neighbour is a pair with a count.*
13. **`Hierarchy` is the map's tree of regions** (`m.hierarchy`), with kinds
    `community`, `subdivision`, `aggregate`; the levels of the algorithm are reached
    through `Communities` (`level=`). *`DESIGN.md` D16 separates the two outputs; the
    kinds use the words of `SPEC.md` (#63 says "subcommunity" for the second).*
14. **`eg.read` also takes a table**, and keeps small inputs in memory (below 100 MB †)
    unless `out=` is given. *`api.md` Choice 1 recommends exactly this split; #83 and #95
    ask for tables in.*

**B. Names taken from issue files** (marked ‡; not shown in an approved preview)

| Name | From | Kept because |
|---|---|---|
| `eg.from_edges`, `eg.from_scipy`, `g.save` | #84 | Story 10 needs the first; the names are already in the plan. |
| `g.as_of` | #94 | A one-sided window; named in the plan. |
| `g.has_edge`, `g.common_neighbors`, `g.edges_between` | #90 | Named in the plan; they follow the short-name rule. |
| `g.to_networkx`, `g.to_igraph`, `g.to_scipy` | #85 | Story 7 needs the first. |
| `InvalidInput` | #81 | |
| `--threads`, `--temp-dir`, `--seed`, `--deterministic`, `--fast`, `everygraph export`, `everygraph verify` | #98 | |

**C. Names added here** (in neither a preview nor an issue; each is the smallest name
that makes a previewed feature usable)

| Name | Needed for |
|---|---|
| `g.nodes()`, `g.edges()`, `g.columns`, `g.file`, `g.info()` | Getting data and metadata out; `info` mirrors the command. `file`, because `g.path` is taken by the path query. |
| `c.nodes()`, `c.levels`, `c.modularity`, `Community` | The membership table of story 1 has no call in the previews (`c.table()` is per community); `nodes()` follows `m.selection.nodes()`. |
| `Table.write`, `.meta`, `.columns`, `.num_rows`, `.approximate`, `.to_arrow`, `ids=` | § 2.5, § 2.8 and the `--out` of the command. |
| `Path.found`, `.hops`, `.nodes()`, `.edges()`; `Selection.kind`; `Map.file`, `.levels`, `.hierarchy`; `Hierarchy.*`; `Plan.fits`, `.to_dict()` | The object model the issue asks for. |
| `progress=`, `temp_dir=`, `out=`, `reveal=`, `strong=`, `method=`, `objective=`, `iterations=`, `personalize=`, `damping=`, `tol=`, `max_iter=`, `events=`, `max_edges=` | Parameters the implementing issues describe without naming (#47, #48, #57, #58, #81, #85). |
| `InsufficientDisk`, `InvalidExpression`, `UnknownNode`, `Unsupported`, `CorruptFile`, `EverygraphError` | § 2.7. |
| `--window`, `--edges`, `--nodes`, `--avoid`, `--avoid-hubs`, `--reveal`, `--deep`, `--no-weight`, `--directed`, `--undirected`, `--strong` | Item 9. |
