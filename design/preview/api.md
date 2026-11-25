# Preview — the Python API

**This is a proposal to react to, not working software.** Outputs are invented; they show
shape and tone, not measurements. Each numbered **Choice** has a recommendation (★) and at
least one alternative. Answers go to `design/DECISIONS.md`.

---

## 1. Opening a graph

```python
import everygraph as eg

g = eg.read("transfers.parquet", source="from", target="to",
            weight="value", time="block_time", directed=True)
```

Before anything long runs, it says what it is about to do:

```text
transfers.parquet · 38.2 GB · 1,830,402,118 rows
  ids        hex addresses (20 bytes)          212,418,907 distinct (estimated)
  edges      directed · duplicates kept as events · 4,201 self-loops will be dropped
  columns    value → weight (decimal) · block_time → time · 3 more ignored: gas, nonce, input

plan       build on disk (graph is 4.6× the memory budget)
estimate   3 min 40 s · peak memory 5.9 GB of 8.0 GB budget · 61 GB scratch · result 9.3 GB
           → transfers.eg

reading       ████████░░░░░░░░  48% of this phase · 1.9 GB/s · 1 min 52 s left
```

Reopening later is instant: `g = eg.open("transfers.eg")`.

> **Choice 1 — where does the prepared graph go?**
> ★ **A.** Next to the input, same name with `.eg` (shown above).
> **B.** In a cache directory (`~/.cache/everygraph/`), invisible unless you ask.
> **C.** Nowhere unless you call `g.save(...)`; small graphs stay in memory.
> *Recommended: A for files above a size threshold, C below it.*

> **Choice 2 — does it ask before a long job?**
> ★ **A.** Never asks; prints the plan and starts. `eg.read(..., dry_run=True)` only plans.
> **B.** Asks for confirmation above a threshold (e.g. 10 minutes or 20 GB of scratch).
> *A is friendlier in scripts and notebooks; B protects against surprises.*

---

## 2. Asking questions about the whole graph

```python
pr = g.pagerank()
pr.top(5)
```

```text
shape: (5, 3)
┌──────────────────────────────────────────────┬────────────┬──────┐
│ node                                         │ pagerank   │ rank │
├──────────────────────────────────────────────┼────────────┼──────┤
│ 0x28c6c06298d514db089934071355e5743bf21d60   │ 0.00412907 │    1 │
│ 0xdac17f958d2ee523a2206206994597c13d831ec7   │ 0.00388114 │    2 │
│ 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48   │ 0.00301775 │    3 │
│ 0x21a31ee1afc51d94c2efccaa2092ad1028285549   │ 0.00214466 │    4 │
│ 0x7a250d5630b4cf539739df2c5dacb4c659f2488d   │ 0.00198203 │    5 │
└──────────────────────────────────────────────┴────────────┴──────┘
```

```python
g.components()          # node → component, plus .sizes()
g.kcore()               # node → core number
g.triangles()           # node → triangles, clustering coefficient
g.degrees()             # node → in, out, total, weighted
g.bfs("0x28c6…1d60")    # node → distance from a start node
```

> **Choice 3 — how are algorithms called?**
> ★ **A.** Methods on the graph: `g.pagerank()`. Short, discoverable with Tab.
> **B.** Functions: `eg.pagerank(g)`. Familiar from NetworkX (`nx.pagerank(G)`).
> **C.** Grouped: `g.centrality.pagerank()`, `g.paths.shortest(...)`. Tidy when there are
> a hundred algorithms, more typing when there are fifteen.

> **Choice 4 — what comes back?**
> ★ **A.** A small Everygraph table (shown above) with a few graph-aware helpers
> (`.top(n)`, `.on_map()`), which *is* an Arrow table: `pl.from_arrow(pr)`,
> `pr.to_pandas()`, `duckdb.sql("select … from pr")` all work without copying.
> **B.** A Polars DataFrame directly (Polars becomes a required dependency).
> **C.** NumPy arrays aligned with internal node numbers — fastest, least friendly.

> **Choice 5 — identifiers in results**
> ★ **A.** Your original ids (addresses, usernames), as above.
> **B.** Internal integers by default, original ids on request — less memory, less readable.

---

## 3. Communities

```python
c = g.communities()            # deterministic: same answer every run, on any machine
c
```

```text
Communities of transfers.eg — Leiden, resolution 1.0, deterministic
  31,960 communities · modularity 0.6033 · 0 disconnected
  levels   212,418,907 nodes → 17,704,220 → 1,551,093 → 153,402 → 31,960
  largest  18,204,551 nodes (8.57%) · median 37 · 12,877 singletons
  took     2 min 11 s
```

```python
c.table().head(3)
```

```text
┌───────────┬────────────┬────────────────┬───────────────┬──────────┐
│ community │ nodes      │ internal_edges │ edges_out     │ share    │
├───────────┼────────────┼────────────────┼───────────────┼──────────┤
│         0 │ 18,204,551 │    402,118,330 │    96,410,227 │ 8.57%    │
│         1 │ 11,930,204 │    188,300,412 │    71,002,118 │ 5.62%    │
│         2 │  9,410,877 │    151,902,006 │    40,331,905 │ 4.43%    │
└───────────┴────────────┴────────────────┴───────────────┴──────────┘
```

```python
c.of("0x28c6…1d60")            # which community, at every level
g.subgraph(c[0]).pagerank()    # any analysis inside one community
```

> **Choice 6 — speed or repeatability by default?**
> ★ **A.** Repeatable by default (`deterministic=True`): identical result at any number of
> cores. In the lab this was 2.4–3× slower than the fast mode at 16 threads.
> **B.** Fast by default; repeatable on request.
> *This is a real trade-off: A is what a paper or an investigation needs, B is what a first
> look wants.*

> **Choice 7 — what is it called?**
> ★ **A.** `g.communities()` — says what you get. The method is a parameter.
> **B.** `g.leiden()` — says how. Familiar to specialists.

---

## 4. Asking about specific nodes

```python
g.neighbors("0x28c6…1d60", direction="out", limit=5)
g.khop("0x28c6…1d60", k=2, limit=10_000)         # stops at 10,000 nodes and says so
p = g.path("0x4f3a…91bc", "0x9e12…07aa", max_hops=5, avoid=g.top("degree", 100))
p
```

```text
Path · 4 hops · found in 1.3 ms
  0x4f3a…91bc ──  12.400 ETH · 2024-03-02 ──▶ 0x77e0…c1d4
  0x77e0…c1d4 ──   3.150 ETH · 2024-03-02 ──▶ 0xb81c…5520
  0xb81c…5520 ──   3.149 ETH · 2024-03-04 ──▶ 0x02fd…a9e3
  0x02fd…a9e3 ──   3.100 ETH · 2024-03-09 ──▶ 0x9e12…07aa
  avoided 100 hub nodes · 2 other shortest paths exist: p.alternatives()
```

> **Choice 8 — when a question is too big to answer fully**
> ★ **A.** Every such call takes a `limit` (nodes) and a `timeout` (seconds) with sensible
> defaults and returns a partial answer marked `truncated=True`. ("Budget" is kept for
> memory only, so the word means one thing.)
> **B.** No default limit; it runs as long as it takes unless you set one.

---

## 5. Looking at part of the graph

```python
march = g.window("2024-03-01", "2024-03-31")
big   = g.filter(edges="value > 10")
both  = march & big

both.num_edges        # 4,118,306
both.communities()    # everything above works on a view
```

> **Choice 9 — how are filters written?**
> ★ **A.** Short text, as above: `edges="value > 10"`. Quick to type; errors appear when
> the text is parsed.
> **B.** Expression objects, Polars-style: `g.filter(edges=eg.col("value") > 10)`. More
> typing, autocompletion and earlier errors.
> **C.** Both.

---

## 6. When it cannot be done

```python
g.communities(memory="4GB")
```

```text
InsufficientBudget: communities() needs 5.1 GB for per-node state; the budget is 4.0 GB.
  212,418,907 nodes × 24 bytes = 5.1 GB. The edges stay on disk either way.

  Ways forward:
    g.communities(memory="6GB")       5.1 GB of state + buffers: the smallest that works
    g.communities(precision="low")    needs 3.4 GB; slightly lower quality
```

> **Choice 10 — tone of messages**
> ★ **A.** As above: what, why with the arithmetic, and the ways out.
> **B.** Terse one-liners, details behind `eg.explain_last_error()`.

---

## 7. The map

```python
m = g.map()            # uses the communities; lays them out; builds the tiles
m                      # shows the interactive map in the notebook
m.export("transfers.egmap")              # one file, hostable anywhere static
m.selection.nodes()                      # what you clicked, back in Python
m.highlight(pr.top(100))                 # push a result onto the map
```

> **Choice 11 — how much does `g.map()` do on its own?**
> ★ **A.** Everything missing (communities, hierarchy, layout, tiles), after printing the
> plan. One line from file to map.
> **B.** Only the map; it fails with a clear message if communities were not computed.

---

## 8. Coming from NetworkX

```python
import networkx as nx
G = eg.nx.read_edgelist("edges.csv")     # loaded by Everygraph
nx.pagerank(G)                            # runs in Everygraph, returns what NetworkX returns
```

> **Choice 12 — how prominent should this be?**
> ★ **A.** A separate optional package (`pip install nx-everygraph`), mentioned in the docs.
> **B.** Built in and advertised on the front page ("your NetworkX code, faster").
> *The lab showed the speed-up is large only when the graph is loaded through us, and
> small (1–3×) when an existing `nx.Graph` has to be converted; B invites a
> disappointment.*

---

## Names, at a glance

| Thing | Proposed | Alternatives |
|---|---|---|
| import alias | `eg` | `evg` |
| open a file | `eg.read()` / `eg.open()` | `eg.load()`, `eg.Graph(path)` |
| filtered graph | "view": `g.filter()`, `g.window()` | `g.where()`, `g.between()` |
| groups found | "communities" | "clusters" |
| areas on the map | "regions" | "communities" everywhere |
| memory limit | `memory="8GB"` | `budget=`, `max_memory=` |
| k-core | `g.kcore()` | `g.core_numbers()` |
| connected components | `g.components()` | `g.connected_components()` |

> **Choice 13 — short names or NetworkX-style long names?**
> ★ **A.** Short (`components`, `kcore`). **B.** Long and explicit
> (`connected_components`, `core_number`), matching what NetworkX users already type.
