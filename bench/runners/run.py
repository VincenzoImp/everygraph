# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Thin comparison runners for everygraph-bench.

    python run.py TOOL TASK --graph DIR [--threads N] [--work DIR] [...]

One process runs one task of one tool on a graph in the lab's glab-csr-v0 format
(offsets.u64, targets.u32, symmetric, sorted, no self-loops) and prints one JSON line:

    {"tool", "version", "task", "algorithm", "options", "graph", "threads",
     "load_seconds", "seconds", "result": {...}}

`algorithm` names exactly what was called, with the options that change what is
computed: a "triangle count" baseline once turned out to be a clustering coefficient
in a slow mode (CLAUDE.md § 3). `seconds` times the tool's call alone, in process:
loading and conversion are in `load_seconds`, and turning the tool's answer into
`result` (summing scores, computing modularity) is after the clock stops. The harness
measures the whole process around it and stores `tool`, `version`, `algorithm` and
`options` in every row. Thread counts come from the environment the spec sets
(OMP_NUM_THREADS before any import) and from `--threads`.

`--trials K` runs the call K times in the process; `seconds` is their median,
`seconds_min` the fastest. `--dump FILE` writes the result for `validate`.

Also `python run.py convert mtx --graph DIR --out FILE`: MatrixMarket for GVE; `sg`,
`wsg`, `wsg-sym` and `wsg-sym64`: GAP's serialized graphs, the last three weighted:
`wsg` with the costs of `edge_map_bench --kernel sssp` (integers 1 to 255, a function of
the entry), the `-sym` ones with the same draw on the undirected pair (`pair_costs`), as
int32 or f64; `ldbc-csr --input DATASET --out DIR`: a weighted LDBC data set as a CSR directory
with its costs; `gbbs`: GBBS's binary symmetric format (also Parallel-KCore's and
PASGAL's `.bin`). And `python run.py gap|gbbs|pkc|pasgal TASK --graph DIR --work DIR
[--trees DIR]`: the C++ reference tools that bench/references/build.sh builds, on the
graph files made in `--work` (`GAP_TASKS`, `GBBS_TASKS`, `UCR`); `seconds` is the
tool's own kernel time (the median trial, or the mean over the sources of `--sources`).
`python run.py validate --graph DIR --work DIR ...` checks every tool's result against
Everygraph's (`validate`).
"""

import argparse
import json
import os
import re
import statistics
import sys
import time

import numpy as np

QUERY_SEED = 42  # the lab's query set (lab/f2-python/scripts/engines/make_queries.py)


# ----------------------------------------------------------------------------- input


class Csr:
    def __init__(self, path, mapped=False):
        meta_path = os.path.join(path, "meta.json")
        self.meta = json.load(open(meta_path)) if os.path.exists(meta_path) else {}
        self.offsets = np.fromfile(os.path.join(path, "offsets.u64"), dtype=np.uint64)
        targets = os.path.join(path, "targets.u32")
        # Mapped for the converters, which stream it, and for validate, which runs the tools
        # beside it; read into the heap for the tools.
        self.targets = np.memmap(targets, dtype=np.uint32, mode="r") if mapped else np.fromfile(targets, dtype=np.uint32)
        self.n = len(self.offsets) - 1
        self.degrees = np.diff(self.offsets).astype(np.int64)
        self.name = os.path.basename(os.path.normpath(path))

    def sources(self):
        return np.repeat(np.arange(self.n, dtype=np.uint32), self.degrees)

    def undirected_edges(self):
        """Each edge once, as (u, v) with u < v."""
        src = self.sources()
        keep = src < self.targets
        return src[keep], self.targets[keep]

    def hub(self):
        return int(self.degrees.argmax())

    def queries(self):
        rng = np.random.default_rng(QUERY_SEED)
        return {
            "nodes": rng.integers(0, self.n, 200).tolist(),
            "nodes3": rng.integers(0, self.n, 50).tolist(),
            "pairs": rng.integers(0, self.n, (50, 2)).tolist(),
        }


def modularity(csr, labels):
    """Newman modularity of a partition, computed the same way for every tool."""
    labels = np.asarray(labels, dtype=np.int64)
    src = csr.sources()
    two_m = float(len(csr.targets))
    internal = np.count_nonzero(labels[src] == labels[csr.targets])
    volume = np.bincount(labels, weights=csr.degrees.astype(float))
    q = internal / two_m - float(np.square(volume / two_m).sum())
    return {"modularity": round(q, 6), "communities": int(len(np.unique(labels)))}


def latency(fn, inputs, warm=3):
    """Median and p90 in milliseconds of fn over inputs, after a few warm-up calls."""
    for x in inputs[:warm]:
        fn(x)
    times, outputs = [], []
    for x in inputs:
        t = time.perf_counter()
        outputs.append(fn(x))
        times.append((time.perf_counter() - t) * 1e3)
    times.sort()
    return {
        "n": len(inputs),
        "median_ms": statistics.median(times),
        "p90_ms": times[min(len(times) - 1, int(0.9 * len(times)))],
        "checksum": int(sum(o for o in outputs if isinstance(o, int))),
    }


# ----------------------------------------------------------------------------- tools


def networkit(task, csr, args):
    import networkit as nk
    import scipy.sparse as sp

    nk.setNumberOfThreads(args.threads)
    t = time.perf_counter()
    u, v = csr.undirected_edges()
    coo = sp.coo_matrix((np.ones(len(u), dtype=np.float64), (u, v)), shape=(csr.n, csr.n))
    g = nk.GraphFromCoo(coo, n=csr.n, weighted=False, directed=False)
    load = time.perf_counter() - t
    assert g.numberOfEdges() == len(u)

    def pagerank():
        a = nk.centrality.PageRank(g, damp=0.85, tol=1e-30)
        a.maxIterations = args.iterations
        return run_then(a)

    def partition(make):
        # Timed: building and running the algorithm. Not timed: modularity.
        return (lambda: run_then(make()),
                lambda a: modularity(csr, a.getPartition().getVector()))

    tasks = {
        "pagerank": (f"networkit.centrality.PageRank(damp=0.85, tol=1e-30), maxIterations={args.iterations}, float64",
                     pagerank, lambda a: {"iterations": a.numberOfIterations()}),
        "components": ("networkit.components.ConnectedComponents (sequential)",
                       lambda: run_then(nk.components.ConnectedComponents(g)),
                       lambda a: {"components": a.numberOfComponents()}),
        "parallel_components": ("networkit.components.ParallelConnectedComponents",
                                lambda: run_then(nk.components.ParallelConnectedComponents(g)),
                                lambda a: {"components": a.numberOfComponents()}),
        "kcore": ("networkit.centrality.CoreDecomposition",
                  lambda: run_then(nk.centrality.CoreDecomposition(g)),
                  lambda a: {"max_core": a.maxCoreNumber()}),
        "bfs": ("networkit.distance.BFS(source = highest-degree node, storePaths=False)",
                lambda: run_then(nk.distance.BFS(g, csr.hub(), storePaths=False)),
                lambda a: {"source": csr.hub()}),
        "bfs64": ("networkit.distance.BFS(storePaths=False) from each source of --sources, mean per source",
                  lambda: each_source(lambda s: run_then(nk.distance.BFS(g, s, storePaths=False)) and None,
                                      read_sources(args))),
        "bfs1": ("networkit.distance.BFS(storePaths=False) from the first source of --sources",
                 lambda: run_then(nk.distance.BFS(g, read_sources(args)[0], storePaths=False)),
                 lambda a: {"source": read_sources(args)[0]}),
        # Per-edge triangle counts; each triangle is on three edges. Timed: the edge index
        # the algorithm requires, and run(). Not timed: scores(), which copies 117 M floats
        # into a Python list on orkut.
        "triangles": ("networkit.Graph.indexEdges() + networkit.sparsification.TriangleEdgeScore.run(), "
                      "scores summed / 3 after the clock (an exact triangle count)",
                      lambda: (g.indexEdges(), run_then(nk.sparsification.TriangleEdgeScore(g)))[1],
                      lambda a: {"triangles": int(sum(a.scores()) // 3)}),
        "plm": ("networkit.community.PLM(refine=False) (parallel Louvain)",
                *partition(lambda: nk.community.PLM(g, refine=False))),
        "leiden": ("networkit.community.ParallelLeiden(iterations=2), defaults otherwise",
                   *partition(lambda: nk.community.ParallelLeiden(g, iterations=2))),
    }
    unreached = np.finfo(np.float64).max  # NetworKit's distance of an unreached node
    dumps = {
        "pagerank": lambda a: np.asarray(a.scores(), dtype=np.float64),
        "components": lambda a: np.asarray(a.getPartition().getVector(), dtype=np.uint32),
        "parallel_components": lambda a: np.asarray(a.getPartition().getVector(), dtype=np.uint32),
        "kcore": lambda a: np.asarray(a.scores(), dtype=np.float64).astype(np.uint32),
        "bfs1": lambda a: np.where(np.asarray(a.getDistances()) >= unreached, np.inf, a.getDistances()),
    }
    return nk.__version__, load, tasks, "networkit.GraphFromCoo, undirected, unweighted, from the CSR", dumps


def run_then(a):
    a.run()
    return a


def need_edges(args):
    if not args.edges:
        raise SystemExit("this tool loads a text edge list: pass --edges FILE (made by `run.py convert edges`)")
    return args.edges


def igraph(task, csr, args):
    import igraph as ig

    # From text, as F2 did: the numpy 2-D constructor needed more than 6 GB on lj.
    edges = need_edges(args)
    t = time.perf_counter()
    g = ig.Graph.Read_Edgelist(edges, directed=False)
    if g.vcount() < csr.n:
        g.add_vertices(csr.n - g.vcount())
    load = time.perf_counter() - t
    q = csr.queries()

    def queries():
        def sp(p):
            # BFS with early exit at the target; distances() would run a full BFS.
            path = g.get_shortest_path(p[0], p[1])
            return len(path) - 1 if path else -1
        return {
            "neighbors": latency(lambda x: len(g.neighbors(x)), q["nodes"]),
            "hop2": latency(lambda x: g.neighborhood_size(x, order=2) - 1, q["nodes"]),
            "shortest_path": latency(sp, q["pairs"]),
        }

    tasks = {
        # Not 20 power iterations: PRPACK solves the linear system exactly.
        "pagerank": ("igraph.Graph.pagerank(damping=0.85), PRPACK (exact solve, not iterations), float64",
                     lambda: g.pagerank(damping=0.85), lambda pr: {"max": max(pr)}),
        "components": ("igraph.Graph.connected_components", lambda: g.connected_components(),
                       lambda c: {"components": len(c)}),
        "kcore": ("igraph.Graph.coreness", lambda: g.coreness(), lambda c: {"max_core": int(max(c))}),
        "bfs": ("igraph.Graph.bfs(source = highest-degree node)", lambda: {"reached": len(g.bfs(csr.hub())[0])}),
        "bfs64": ("igraph.Graph.bfs (returns Python lists of n) from each source of --sources, mean per source",
                  lambda: each_source(lambda s: len(g.bfs(s)[0]), read_sources(args))),
        "bfs1": ("igraph.Graph.bfs(source = the first source of --sources), the call bfs64 times",
                 lambda: g.bfs(read_sources(args)[0]), lambda r: {"source": read_sources(args)[0]}),
        # No triangles task: igraph's Python API lists them (a tuple each, 177 M on lj)
        # rather than counting, and transitivity is a different quantity.
        "multilevel": ("igraph.Graph.community_multilevel (Louvain)", lambda: g.community_multilevel(),
                       lambda c: modularity(csr, c.membership)),
        "leiden": ("igraph.Graph.community_leiden(objective_function='modularity', n_iterations=2)",
                   lambda: g.community_leiden(objective_function="modularity", n_iterations=2),
                   lambda c: modularity(csr, c.membership)),
        "queries": ("neighbors(), neighborhood_size(order=2), get_shortest_path() on the lab's seed-42 query set", queries),
    }
    dumps = {
        "components": lambda c: np.asarray(c.membership, dtype=np.uint32),
        "kcore": lambda c: np.asarray(c, dtype=np.uint32),
        "bfs1": lambda r: igraph_parents(r[2], read_sources(args)[0]),
    }
    return ig.__version__, load, tasks, "igraph.Graph.Read_Edgelist(text, each edge once), undirected", dumps


def igraph_parents(parents, source):
    """igraph's BFS parents as a parent array: the root its own parent, -1 where unreached
    (igraph gives -1 for the root and a smaller negative number where unreached)."""
    p = np.asarray(parents, dtype=np.int64)
    p[p < 0] = -1
    p[source] = source
    return p.astype(np.int32)


def rustworkx(task, csr, args):
    import rustworkx as rx

    edges = need_edges(args)
    t = time.perf_counter()
    g = rx.PyGraph.read_edge_list(edges, deliminator=" ")
    while len(g) < csr.n:
        g.add_node(None)
    load = time.perf_counter() - t
    q = csr.queries()

    def khop(x, k):
        # rustworkx has no k-hop count for PyGraph: a Python BFS over native neighbors().
        seen, frontier = {x}, [x]
        for _ in range(k):
            nxt = []
            for a in frontier:
                for b in g.neighbors(a):
                    if b not in seen:
                        seen.add(b)
                        nxt.append(b)
            frontier = nxt
        return len(seen) - 1

    def sp(p):
        paths = rx.graph_dijkstra_shortest_paths(g, p[0], target=p[1], default_weight=1.0)
        return len(paths[p[1]]) - 1 if p[1] in paths else -1

    tasks = {
        "components": ("rustworkx.connected_components", lambda: {"components": len(rx.connected_components(g))}),
        "kcore": ("rustworkx.core_number", lambda: {"max_core": max(rx.core_number(g).values())}),
        "bfs": ("rustworkx.bfs_successors(source = highest-degree node)", lambda: {"levels": len(rx.bfs_successors(g, csr.hub()))}),
        "queries": ("neighbors(); 2-hop by a Python BFS over neighbors(); graph_dijkstra_shortest_paths(weight 1, target)",
                    lambda: {"neighbors": latency(lambda x: len(g.neighbors(x)), q["nodes"]),
                             "hop2": latency(lambda x: khop(x, 2), q["nodes"]),
                             "shortest_path": latency(sp, q["pairs"])}),
    }
    return rx.__version__, load, tasks, "rustworkx.PyGraph.read_edge_list(text, each edge once), undirected", {}


def duckdb(task, csr, args):
    import duckdb as ddb

    if not args.input:
        raise SystemExit("duckdb ingest needs --input (a SNAP text edge list, .txt or .txt.gz)")
    work = args.work or "."
    tmp = os.path.join(work, "duckdb")
    os.makedirs(tmp, exist_ok=True)
    con = ddb.connect(os.path.join(tmp, "w.duckdb"))
    con.execute(f"SET threads={args.threads}; SET memory_limit='{args.memory_limit}'; "
                f"SET temp_directory='{tmp}/tmp'; SET preserve_insertion_order=false")

    def ingest():
        steps = {}

        def step(name, sql):
            t = time.perf_counter()
            con.execute(sql)
            steps[name] = round(time.perf_counter() - t, 3)

        # SNAP text: '#' comment lines, then tab-separated pairs. The sniffer rejects it,
        # so the dialect is given (auto_detect=false).
        step("read", f"CREATE TABLE e AS SELECT s, t FROM read_csv('{args.input}', delim='\t', header=false, "
                     "comment='#', auto_detect=false, columns={'s': 'BIGINT', 't': 'BIGINT'})")
        step("ids", "CREATE TABLE ids AS SELECT id, (row_number() OVER (ORDER BY id) - 1)::UINTEGER AS k "
                    "FROM (SELECT s id FROM e UNION SELECT t FROM e)")
        step("relabel_symmetrize_dedup_sort", f"""COPY (SELECT DISTINCT a, b FROM (
            SELECT i1.k a, i2.k b FROM e JOIN ids i1 ON e.s = i1.id JOIN ids i2 ON e.t = i2.id WHERE e.s <> e.t
            UNION ALL SELECT i2.k, i1.k FROM e JOIN ids i1 ON e.s = i1.id JOIN ids i2 ON e.t = i2.id WHERE e.s <> e.t)
            ORDER BY a, b) TO '{tmp}/adj.parquet' (FORMAT PARQUET, COMPRESSION ZSTD)""")
        rows = con.execute(f"SELECT count(*) FROM '{tmp}/adj.parquet'").fetchone()[0]
        return {"entries": int(rows), "steps": steps}

    tasks = {
        "ingest": ("DuckDB SQL: read_csv -> dense ids (row_number) -> symmetric, deduplicated, sorted (src, dst) "
                   "-> Parquet ZSTD (lab/a-baselines/duck_ingest.py)", ingest),
    }
    return ddb.__version__, 0.0, tasks, f"text edge list {args.input}; memory_limit {args.memory_limit}", {}


def gfql(task, csr, args):
    import graphistry
    import polars as pl
    from graphistry import e_undirected, n

    t = time.perf_counter()
    u, v = csr.undirected_edges()
    edges = pl.DataFrame({"src": u.astype(np.int64), "dst": v.astype(np.int64)})
    nodes = pl.DataFrame({"id": np.arange(csr.n, dtype=np.int64)})
    g = graphistry.edges(edges, "src", "dst").nodes(nodes, "id")
    load = time.perf_counter() - t
    q = csr.queries()

    def hop(x, k):
        res = g.gfql([n({"id": x}), e_undirected(hops=k), n()], engine="polars")
        return len(res._nodes) - 1

    k = 30  # each hop scans the whole edge frame: the lab used the first 30 inputs
    tasks = {
        "queries": ("graphistry GFQL chain [n({id}), e_undirected(hops=k), n()], engine='polars'; first 30 inputs; "
                    "no shortest-path operator",
                    lambda: {"neighbors": latency(lambda x: hop(x, 1), q["nodes"][:k], warm=1),
                             "hop2": latency(lambda x: hop(x, 2), q["nodes"][:k], warm=1)}),
    }
    version = f"graphistry {graphistry.__version__}, polars {pl.__version__}"
    return version, load, tasks, "polars edge frame, each edge once, from the CSR", {}


def ladybug(task, csr, args):
    import ladybug as lb
    import pyarrow as pa
    import pyarrow.parquet as pq

    work = args.work or "."
    db_path = os.path.join(work, "ladybug.db")
    for p in (db_path, db_path + ".wal", db_path + ".shadow"):
        if os.path.isdir(p):
            import shutil
            shutil.rmtree(p)
        elif os.path.exists(p):
            os.remove(p)
    u, v = csr.undirected_edges()
    pq.write_table(pa.table({"src": u.astype(np.int64), "dst": v.astype(np.int64)}), os.path.join(work, "edges.parquet"))
    pq.write_table(pa.table({"id": np.arange(csr.n, dtype=np.int64)}), os.path.join(work, "nodes.parquet"))
    db = lb.Database(db_path, buffer_pool_size=3 << 30, max_num_threads=args.threads)
    c = lb.Connection(db, num_threads=args.threads)
    t = time.perf_counter()
    c.execute("CREATE NODE TABLE N(id INT64 PRIMARY KEY)")
    c.execute("CREATE REL TABLE E(FROM N TO N)")
    c.execute(f"COPY N FROM '{work}/nodes.parquet'")
    c.execute(f"COPY E FROM '{work}/edges.parquet'")
    c.execute("CHECKPOINT")
    load = time.perf_counter() - t
    c.set_query_timeout(10000)
    q = csr.queries()

    def query(stmt):
        prepared = c.prepare(stmt)
        return lambda params: c.execute(prepared, params).get_all()

    nb = query("MATCH (a:N {id:$x})-[:E]-(b:N) RETURN b.id")
    k2 = query("MATCH (a:N {id:$x})-[:E* SHORTEST 1..2]-(b:N) RETURN count(DISTINCT b.id)")
    spq = query("MATCH p=(a:N {id:$x})-[:E* SHORTEST 1..30]-(b:N {id:$y}) RETURN length(p)")

    def sp(p):
        if p[0] == p[1]:
            return 0
        out = spq({"x": p[0], "y": p[1]})
        return out[0][0] if out else -1

    tasks = {
        "queries": ("Cypher on LadybugDB: (a)-[:E]-(b); [:E* SHORTEST 1..2] distinct count; [:E* SHORTEST 1..30] length; "
                    "query timeout 10 s",
                    lambda: {"neighbors": latency(lambda x: len(nb({"x": x})), q["nodes"]),
                             "hop2": latency(lambda x: k2({"x": x})[0][0], q["nodes"]),
                             "shortest_path": latency(sp, q["pairs"])}),
    }
    version = getattr(lb, "__version__", "unknown")
    return version, load, tasks, "on-disk node and rel tables (each edge once), COPY FROM Parquet; load = COPY + CHECKPOINT", {}


TOOLS = {"networkit": networkit, "igraph": igraph, "rustworkx": rustworkx, "duckdb": duckdb, "gfql": gfql,
         "ladybug": ladybug}


# ----------------------------------------------------------------------------- main


def uniform_costs(m, start=0):
    """`cost(e, false)` of crates/everygraph-kernels/examples/edge_map_bench.rs, for the
    entries start .. start + m."""
    z = np.arange(start, start + m, dtype=np.uint64) + np.uint64(0x9E3779B97F4A7C15)
    z = (z ^ (z >> np.uint64(30))) * np.uint64(0xBF58476D1CE4E5B9)
    z = (z ^ (z >> np.uint64(27))) * np.uint64(0x94D049BB133111EB)
    z ^= z >> np.uint64(31)
    return (np.uint64(1) + z % np.uint64(255)).astype(np.int32)


def mix(z):
    """SplitMix64's finalizer, as edge_map_bench's `cost` applies it."""
    z = z + np.uint64(0x9E3779B97F4A7C15)
    z = (z ^ (z >> np.uint64(30))) * np.uint64(0xBF58476D1CE4E5B9)
    z = (z ^ (z >> np.uint64(27))) * np.uint64(0x94D049BB133111EB)
    return z ^ (z >> np.uint64(31))


def pair_costs(u, v):
    """The cost of the undirected pair {u, v}, the same both ways: an integer from 1 to 255
    drawn from min(u, v) * 2^32 + max(u, v). bench/specs/references.toml gives every tool
    these costs (bench/references/eg-driver and ucr_driver.cc compute the same), so a tool
    may use that the graph is symmetric (PASGAL's pull step)."""
    u, v = np.asarray(u, dtype=np.uint64), np.asarray(v, dtype=np.uint64)
    with np.errstate(over="ignore"):
        key = (np.minimum(u, v) << np.uint64(32)) | np.maximum(u, v)
        return (np.uint64(1) + mix(key) % np.uint64(255)).astype(np.int32)


CHUNK = 1 << 26  # entries per write: the converters stream Friendster's 3.6 G entries


# GAP's NodeWeight<int32, WeightT> as stored: int32 costs, or f64 costs after 4 bytes of
# padding (the struct's alignment).
WNODE = {"int32": np.dtype([("v", "<i4"), ("w", "<i4")]),
         "f64": np.dtype([("v", "<i4"), ("pad", "<i4"), ("w", "<f8")])}


def write_targets(csr, f, weights=None, pairs=False):
    """The targets, or (target, cost) records: entry costs (`uniform_costs`) or, with
    `pairs`, the costs of the undirected pairs (`pair_costs`)."""
    m = len(csr.targets)
    with np.errstate(over="ignore"):
        for lo in range(0, m, CHUNK):
            t = np.asarray(csr.targets[lo:lo + CHUNK])
            if weights:
                rec = np.zeros(len(t), dtype=WNODE[weights])
                rec["v"] = t.view(np.int32)
                if pairs:
                    entries = np.arange(lo, lo + len(t), dtype=np.uint64)
                    u = np.searchsorted(csr.offsets, entries, side="right") - 1
                    rec["w"] = pair_costs(u, t)
                else:
                    rec["w"] = uniform_costs(len(t), lo)
                rec.tofile(f)
            else:
                t.tofile(f)


def gap_serialized(csr, weights, out, pairs=False):
    """GAP's .sg / .wsg (src/reader.h, ReadSerializedGraph), undirected: a bool, the
    number of entries and of nodes as int64, the int64 offsets, then the int32 targets,
    or (target, weight) records (`WNODE`)."""
    m = len(csr.targets)
    tmp = out + ".part"
    with open(tmp, "wb") as f:
        f.write(np.array([0], dtype=np.uint8).tobytes())
        f.write(np.array([m, csr.n], dtype=np.int64).tobytes())
        csr.offsets.astype(np.int64).tofile(f)
        write_targets(csr, f, weights, pairs)
    os.replace(tmp, out)


def gbbs_binary(csr, out):
    """GBBS's binary symmetric graph (gbbs/graph_io.cc, parse_unweighted_graph with -b,
    built with GBBSLONG): n, m and the file size as int64, the n + 1 offsets as uint64,
    then the uint32 targets: the CSR's own arrays behind a 24-byte header."""
    m = len(csr.targets)
    tmp = out + ".part"
    with open(tmp, "wb") as f:
        f.write(np.array([csr.n, m, 24 + 8 * (csr.n + 1) + 4 * m], dtype=np.int64).tobytes())
        csr.offsets.tofile(f)
        write_targets(csr, f)
    os.replace(tmp, out)


def ldbc_csr(dataset, out):
    """An undirected, weighted LDBC Graphalytics data set (`<g>.v`, `<g>.e` with "u v w"
    lines) as a CSR directory: `offsets.u64`, `targets.u32` and `costs.f64`, one cost per
    entry. Vertex ids become dense indices in increasing id order, as the oracle's
    `ldbc::read` does; each edge is listed both ways."""
    import pyarrow.csv as pc

    name = os.path.basename(os.path.normpath(dataset))
    opts = pc.ReadOptions(autogenerate_column_names=True)
    parse = pc.ParseOptions(delimiter=" ")
    ids = np.sort(pc.read_csv(os.path.join(dataset, f"{name}.v"), opts, parse).column(0).to_numpy())
    e = pc.read_csv(os.path.join(dataset, f"{name}.e"), opts, parse)
    u = np.searchsorted(ids, e.column(0).to_numpy()).astype(np.uint32)
    v = np.searchsorted(ids, e.column(1).to_numpy()).astype(np.uint32)
    w = e.column(2).to_numpy().astype(np.float64)
    src, dst, cost = np.concatenate([u, v]), np.concatenate([v, u]), np.concatenate([w, w])
    order = np.lexsort((dst, src))
    offsets = np.zeros(len(ids) + 1, dtype=np.uint64)
    np.cumsum(np.bincount(src, minlength=len(ids)), out=offsets[1:])
    os.makedirs(out, exist_ok=True)
    offsets.tofile(os.path.join(out, "offsets.u64"))
    dst[order].tofile(os.path.join(out, "targets.u32"))
    cost[order].tofile(os.path.join(out, "costs.f64"))


def convert(csr, kind, out):
    """`mtx`: MatrixMarket 'pattern symmetric', each edge once, 1-based (as
    lab/a-baselines/csr2mtx.py, for GVE). `edges`: "u v" lines, each edge once, 0-based
    (for igraph and rustworkx, as lab/f2-python). `sg`, `wsg`: GAP's (above), `wsg`
    with entry costs (bench/specs/kernels.toml); `wsg-sym` and `wsg-sym64` with pair costs,
    int32 or f64 (for a GAP built with WeightT = double)."""
    kinds = {"sg": (None, False), "wsg": ("int32", False), "wsg-sym": ("int32", True), "wsg-sym64": ("f64", True)}
    if kind in kinds:
        os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
        gap_serialized(csr, kinds[kind][0], out, kinds[kind][1])
        return
    if kind == "gbbs":
        os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
        gbbs_binary(csr, out)
        return
    import pyarrow as pa
    import pyarrow.csv as pc

    src = csr.sources()
    if kind == "mtx":
        keep = src > csr.targets
        cols = {"i": src[keep].astype(np.int64) + 1, "j": csr.targets[keep].astype(np.int64) + 1}
        header = f"%%MatrixMarket matrix coordinate pattern symmetric\n{csr.n} {csr.n} {int(keep.sum())}\n"
    elif kind == "edges":
        keep = src < csr.targets
        cols = {"u": src[keep].astype(np.int64), "v": csr.targets[keep].astype(np.int64)}
        header = ""
    else:
        raise SystemExit(f"convert {kind}: use mtx, edges, sg, wsg, wsg-sym, wsg-sym64 or gbbs")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    tmp = out + ".part"
    with open(tmp, "wb") as f:
        f.write(header.encode())
        pc.write_csv(pa.table(cols), f, pc.WriteOptions(include_header=False, delimiter=" "))
    os.replace(tmp, out)


# GAP tasks: (binary, graph kind, arguments, how the trial times reduce, algorithm).
# `bfs` and `sssp` start from the hub, best of 5 (bench/specs/kernels.toml). The others
# are those of bench/specs/references.toml: whole-graph kernels 5 trials, their median;
# `bfs64` and `sssp64` from GAP's own 64 seeded non-zero-degree sources (no -r:
# SourcePicker, the list that bench/references/gap_sources.cc prints), one trial each,
# their mean, on the pair costs (`<g>-sym.wsg`). `pr` is pr_spmv.cc (Jacobi) with ScoreT
# changed from float to double, 20 iterations, tol 0: its constants (kDamp, 1.0f / n) stay
# float; `sssp64-f64` is sssp.cc with WeightT changed from int32 to double, on the same
# costs (the references spec builds both).
GAP_TASKS = {
    "bfs": ("bfs", "sg", lambda c: ["-r", str(c.hub()), "-n", "5"], min, "bfs.cc direction-optimizing, from the hub"),
    "sssp": ("sssp", "wsg", lambda c: ["-r", str(c.hub()), "-n", "5", "-d", "2"], min,
             "sssp.cc delta-stepping, -d 2, from the hub"),
    "bfs64": ("bfs", "sg", lambda c: ["-n", "64"], statistics.mean,
              "bfs.cc direction-optimizing, parents, 64 sources of SourcePicker, mean per source"),
    "sssp64": ("sssp", "sym.wsg", lambda c: ["-n", "64", "-d", "2"], statistics.mean,
               "sssp.cc delta-stepping -d 2, int32 pair costs 1..255, 64 sources of SourcePicker, mean per source"),
    "sssp64-f64": ("sssp_f64", "sym-f64.wsg", lambda c: ["-n", "64", "-d", "2"], statistics.mean,
                   "sssp.cc delta-stepping -d 2, WeightT=double, pair costs 1..255, 64 sources of SourcePicker, "
                   "mean per source"),
    "cc": ("cc", "sg", lambda c: ["-n", "5"], statistics.median, "cc.cc Afforest (neighbour rounds 2, sampled skip)"),
    "pr": ("pr_spmv64", "sg", lambda c: ["-n", "5", "-i", "20", "-t", "0"], statistics.median,
           "pr_spmv.cc Jacobi pull, ScoreT=double (kDamp and 1.0f / n stay float), 20 iterations, tol 0"),
    "tc": ("tc", "sg", lambda c: ["-n", "5"], statistics.median,
           "tc.cc ordered count, relabels by degree when its heuristic says so"),
}


def tool_line(tool, version, task, algorithm, cmd, csr, args, times, reduce, **extra):
    print(json.dumps({
        "tool": tool, "version": version, "task": task, "algorithm": algorithm,
        "options": {k: os.environ.get(k) for k in ("OMP_NUM_THREADS", "PARLAY_NUM_THREADS", "OMP_PROC_BIND",
                                                   "OMP_PLACES")} | {"command": cmd[1:],
                                                                      "affinity": sorted(os.sched_getaffinity(0))},
        "graph": csr.name, "threads": args.threads, "seconds": reduce(times), "seconds_min": min(times),
        "trials": len(times), "seconds_all": times, **extra,
    }))


def lines(out, prefix):
    return [float(line.split()[-1]) for line in out.splitlines() if line.startswith(prefix)]


def gap(task, csr, args):
    """GAP's own binaries (SSSP: Δ = 2, its best on these costs among 2, 8 and 32 in an
    exploratory sweep), parsed: its trial times, not the process."""
    import subprocess

    binary, kind, extra, reduce, algorithm = GAP_TASKS[task]
    graph = os.path.join(args.work, f"{csr.name}-{kind}" if kind.startswith("sym") else f"{csr.name}.{kind}")
    tree = tool_tree(args, "gapbs")
    cmd = [os.path.join(tree, binary), "-f", graph, *extra(csr)]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    version = os.path.join(tree, "COMMIT")
    commit = open(version).read().strip() if os.path.exists(version) else subprocess.run(
        ["git", "-C", tree, "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    tool_line("gap", commit, task, algorithm, cmd, csr, args, lines(out, "Trial Time"), reduce,
              load_seconds=sum(lines(out, ("Read Time", "Build Time"))))


# Tasks of bench/references/gbbs_driver.cc: (its -kernel, algorithm).
GBBS_TASKS = {
    "bfs64": ("bfs", "NonDeterministicBFS BFS(G, src), parents, from each source of --sources, mean per source"),
    "cc": ("cc", "WorkEfficientSDB14 connectivity (beta 0.2)"),
    "connectit": ("connectit", "ConnectIt UF-Rem-CAS, k-out sampling, FindAtomicSplit, SplitAtomicOne"),
    "pr": ("pr", "PageRank (dense edgeMapReduce, double), 20 iterations, eps 0; returns p_curr at the cap "
           "(bench/references/build.sh patches the return)"),
    "kcore": ("kcore", "JulienneDBS17 KCore (bucketing, 16 buckets)"),
    "tc": ("tc", "ShunTangwongsan15 Triangle (degree ordering)"),
}


def gbbs(task, csr, args):
    """GBBS's kernels and ConnectIt through bench/references/gbbs_driver.cc, on GBBS's
    binary symmetric format (`convert gbbs`), memory-mapped (-b reads the file in
    place), threads from PARLAY_NUM_THREADS. `seconds`: the median of 5 rounds (GBBS's
    own timer around the kernel), or the mean over the sources."""
    import subprocess

    kernel, algorithm = GBBS_TASKS[task]
    tree = tool_tree(args, "gbbs")
    cmd = [os.path.join(tree, GBBS_DRIVER), "-s", "-b", "-kernel", kernel, "-rounds", "5"]
    if kernel == "bfs":
        cmd += ["-sources", read_sources_path(args)]
    cmd.append(os.path.join(args.work, f"{csr.name}.adj"))
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    searches = lines(out, "### Source Time")
    tool_line("gbbs", open(os.path.join(tree, "COMMIT")).read().strip(), task, algorithm, cmd, csr, args,
              searches or lines(out, "### Round Time"), statistics.mean if searches else statistics.median,
              **({"triangles": int(lines(out, "### Triangles")[0])} if kernel == "tc" else {}))


UCR = {
    "pkc": ("pkc", "Parallel-KCore (Liu et al., SIGMOD'25) KCore::kcore()"),
    "pasgal": ("pasgal", "PASGAL BFS (distances) and rho-stepping SSSP (uint32 pair costs 1..255, symmetric)"),
}


def ucr(tool, task, csr, args):
    """Parallel-KCore and PASGAL through bench/references/ucr_driver.cc, on GBBS's binary
    format, threads from PARLAY_NUM_THREADS; the same reductions as `gbbs`."""
    import subprocess

    name, algorithm = UCR[tool]
    tree = tool_tree(args, name)
    cmd = [os.path.join(tree, "ucr_driver"), "-i", os.path.join(args.work, f"{csr.name}.bin"), "-k", task, "-r", "5"]
    if task in ("bfs", "sssp"):
        cmd += ["-q", read_sources_path(args)]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    searches = lines(out, "### Source Time")
    commit = open(os.path.join(tree, "COMMIT")).read().strip()
    tool_line(tool, commit, task, algorithm, cmd, csr, args, searches or lines(out, "### Round Time"),
              statistics.mean if searches else statistics.median)


GBBS_DRIVER = "benchmarks/Connectivity/WorkEfficientSDB14/gbbs_driver"


def tool_tree(args, name):
    """A tool's build: TREES/NAME (bench/references/build.sh), else WORK/NAME."""
    return os.path.join(args.trees or args.work, name)


def read_sources_path(args):
    if not args.sources:
        raise SystemExit("this task needs --sources FILE (made by bench/references/gap_sources.cc)")
    return args.sources


# ----------------------------------------------------------------------------- validate


def canonical_labels(labels):
    """A partition as the label of each node's component numbered by first appearance:
    equal for two labelings of the same partition."""
    _, inverse = np.unique(np.asarray(labels), return_inverse=True)
    first = np.full(inverse.max() + 1, len(inverse), dtype=np.int64)
    np.minimum.at(first, inverse, np.arange(len(inverse)))
    rank = np.empty_like(first)
    rank[np.argsort(first, kind="stable")] = np.arange(len(first))
    return rank[inverse]


def depths_from_parents(csr, parents, source, unreached):
    """BFS depths from a parent array (the root its own parent), checking that every
    parent is a neighbour: a valid tree of shortest paths has the depths of the BFS
    distances."""
    p = np.asarray(parents).astype(np.int64)
    reached = p != unreached if unreached is not None else p >= 0
    depth = np.full(csr.n, -1, dtype=np.int64)
    depth[source] = 0
    todo = reached.copy()
    todo[source] = False
    while todo.any():
        ready = todo & (depth[np.where(todo, p, source)] >= 0)
        if not ready.any():
            return None, "the parents do not form a tree"
        depth[ready] = depth[p[ready]] + 1
        todo &= ~ready
    # Every (v, parent) must be an entry of v's sorted list: keys (row << 32 | target),
    # sorted because the lists are, searched per block of nodes to bound the memory.
    child = reached.copy()
    child[source] = False
    for a in range(0, csr.n, 1 << 22):
        b = min(csr.n, a + (1 << 22))
        lo, hi = int(csr.offsets[a]), int(csr.offsets[b])
        rows = np.repeat(np.arange(b - a, dtype=np.uint64), csr.degrees[a:b])
        keys = (rows << np.uint64(32)) | np.asarray(csr.targets[lo:hi], dtype=np.uint64)
        vs = np.flatnonzero(child[a:b]).astype(np.uint64)
        want = (vs << np.uint64(32)) | p[a:b][child[a:b]].astype(np.uint64)
        at = np.minimum(np.searchsorted(keys, want), max(len(keys) - 1, 0))
        bad = np.flatnonzero(len(keys) == 0 or keys[at] != want) if len(want) else []
        if len(bad):
            v = a + int(vs[bad[0]])
            return None, f"parent {p[v]} of {v} is not a neighbour"
    return np.where(depth >= 0, depth.astype(np.float64), np.inf), None


def as_distances(a, unreached_at=None):
    d = np.asarray(a).astype(np.float64)
    if unreached_at is not None:
        d[d >= unreached_at] = np.inf
    return d


def validate(csr, args):
    """Runs each tool once with its result written out and compares it with
    Everygraph's: the partition (components), every core number (k-core), every
    distance from the first source (BFS, SSSP: a parent array becomes depths, every
    parent checked to be a neighbour), PageRank by L1 distance (at most 1e-6), triangle
    counts between the tools, and with `--leiden` a modularity floor: every Leiden or
    Louvain run within `LEIDEN_FLOOR` of the best one on the graph (their partitions
    legitimately differ). One JSON line: every check, `invalid` (the number that failed)
    and `skipped_count` (checks whose tool or input is absent; `--without` names the
    tools not run on this graph, which are not counted)."""
    import subprocess
    import tempfile

    w, g = args.work, csr.name
    src = read_sources(args)[0]
    # Only the first source is compared: every tool searches from it alone.
    os.makedirs(os.path.join(w, "validation"), exist_ok=True)
    sources = os.path.join(w, "validation", f"{g}.first-source")
    with open(sources, "w") as f:
        f.write(f"{src}\n")
    gap_bin = tool_tree(args, "gapbs")
    gbbs_bin = os.path.join(tool_tree(args, "gbbs"), GBBS_DRIVER)
    run = [sys.executable, os.path.abspath(__file__)]
    eg = [args.binary, "--graph", args.graph, "--threads", str(args.threads), "--reps", "1", "--sources", sources]
    ucr_args = lambda tree, k: [os.path.join(tool_tree(args, tree), "ucr_driver"), "-i", os.path.join(w, f"{g}.bin"), "-k", k,
                                "-r", "1", "-q", sources]
    gbbs_args = lambda k: [gbbs_bin, "-s", "-b", "-kernel", k, "-rounds", "1", "-sources", sources,
                           os.path.join(w, f"{g}.adj")]
    py = lambda tool, task: run + [tool, task, "--graph", args.graph, "--threads", str(args.threads),
                                   "--sources", sources] + (["--edges", os.path.join(w, f"{g}.edges")]
                                                            if tool == "igraph" else [])
    without = set(filter(None, args.without.split(",")))
    i32_max = np.iinfo(np.int32).max
    u32_max = np.iinfo(np.uint32).max
    # (kernel, tool, command, how the output is written: dtype and its reading; None:
    # the file is passed through the environment as GAP_DUMP, else appended as a flag).
    checks = [
        ("cc", "gap", [os.path.join(gap_bin, "dump_cc"), "-f", os.path.join(w, f"{g}.sg")], np.int32, "labels", None),
        ("cc", "gbbs", gbbs_args("cc"), np.uint32, "labels", "-dump"),
        ("cc", "connectit", gbbs_args("connectit"), np.uint32, "labels", "-dump"),
        ("cc", "cost", [args.cost, "--graph", args.graph, "--kernel", "uf", "--reps", "1"], np.uint32, "labels",
         "--dump"),
        ("kcore", "gbbs", gbbs_args("kcore"), np.uint32, "exact", "-dump"),
        ("kcore", "pkc", ucr_args("pkc", "kcore"), np.uint32, "exact", "-o"),
        ("pr64", "gap", [os.path.join(gap_bin, "dump_pr"), "-f", os.path.join(w, f"{g}.sg"), "-i", "20", "-t", "0"],
         np.float64, "ranks", None),
        ("pr64", "gbbs", gbbs_args("pr"), np.float64, "ranks", "-dump"),
        ("pr64", "cost", [args.cost, "--graph", args.graph, "--kernel", "pr", "--reps", "1"], np.float64, "ranks",
         "--dump"),
        ("bfs", "gap", [os.path.join(gap_bin, "dump_bfs"), "-f", os.path.join(w, f"{g}.sg")], np.int32,
         ("parents", None), None),
        ("bfs", "gbbs", gbbs_args("bfs"), np.uint32, ("parents", u32_max), "-dump"),
        ("bfs", "pasgal", ucr_args("pasgal", "bfs"), np.uint32, ("distances", u32_max), "-o"),
        ("sssp", "gap", [os.path.join(gap_bin, "dump_sssp"), "-f", os.path.join(w, f"{g}-sym.wsg"), "-d", "2"],
         np.int32, ("distances", i32_max // 2), None),
        ("sssp", "gap-f64", [os.path.join(gap_bin, "dump_sssp_f64"), "-f", os.path.join(w, f"{g}-sym-f64.wsg"),
                             "-d", "2"], np.float64, ("distances", np.finfo(np.float64).max / 2), None),
        ("sssp", "pasgal", ucr_args("pasgal", "sssp"), np.uint32, ("distances", u32_max), "-o"),
    ]
    if args.python:
        checks += [
            ("cc", "networkit", py("networkit", "parallel_components"), np.uint32, "labels", "--dump"),
            ("cc", "igraph", py("igraph", "components"), np.uint32, "labels", "--dump"),
            ("kcore", "networkit", py("networkit", "kcore"), np.uint32, "exact", "--dump"),
            ("kcore", "igraph", py("igraph", "kcore"), np.uint32, "exact", "--dump"),
            ("pr64", "networkit", py("networkit", "pagerank"), np.float64, "ranks", "--dump"),
            ("bfs", "networkit", py("networkit", "bfs1"), np.float64, ("distances", np.inf), "--dump"),
            ("bfs", "igraph", py("igraph", "bfs1"), np.int32, ("parents", None), "--dump"),
        ]
    results, skipped = [], []
    with tempfile.TemporaryDirectory(dir=w) as tmp:
        def output(name, cmd, dtype, flag):
            path = os.path.join(tmp, name)
            env = dict(os.environ, GAP_DUMP=path)
            if flag == "-dump":  # GBBS: the graph is the last argument
                cmd = cmd[:-1] + [flag, path, cmd[-1]]
            elif flag:
                cmd = cmd + [flag, path]
            subprocess.run(cmd, check=True, env=env, stdout=subprocess.DEVNULL)
            return np.fromfile(path, dtype=dtype)

        kernels = args.kernels.split(",")
        ours = {k: output(f"eg.{k}", eg + ["--kernel", k], dt, "--dump")
                for k, dt in (("cc", np.uint32), ("kcore", np.uint32), ("pr64", np.float64), ("bfs", np.uint32),
                              ("sssp", np.float64)) if k in kernels}
        checks = [c for c in checks if c[0] in kernels and c[1] not in without]
        convert_ours = {"cc": canonical_labels, "kcore": lambda a: a, "pr64": lambda a: a,
                        "bfs": lambda a: as_distances(a, u32_max), "sssp": as_distances}
        base = {k: convert_ours[k](a) for k, a in ours.items()}
        for kernel, tool, cmd, dtype, how, flag in checks:
            absent = [a for a in cmd if a.startswith(w) and not os.path.exists(a)]
            if absent:
                skipped.append(f"{tool} {kernel}: {absent[0]} absent")
                continue
            try:
                got = output(f"{tool}.{kernel}", cmd, dtype, flag)
            except subprocess.CalledProcessError as e:
                results.append({"kernel": kernel, "tool": tool, "valid": False, "detail": f"exit {e.returncode}"})
                continue
            detail = None
            if len(got) != csr.n:
                ok, detail = False, f"{len(got)} values for {csr.n} nodes"
            elif how == "labels":
                c = canonical_labels(got)
                ok = bool(np.array_equal(c, base["cc"]))
                detail = f"{int(c.max()) + 1} components (ours {int(base['cc'].max()) + 1})"
            elif how == "exact":
                ok = bool(np.array_equal(got, base[kernel]))
                detail = f"max core {int(got.max())} (ours {int(base[kernel].max())})"
            elif how == "ranks":
                l1 = float(np.abs(got - base["pr64"]).sum())
                ok, detail = l1 <= 1e-6, f"L1 distance to ours {l1:.3g}"
            else:
                form, unreached = how
                if form == "parents":
                    d, detail = depths_from_parents(csr, got, src, unreached)
                else:
                    d = as_distances(got, unreached)
                ok = d is not None and bool(np.array_equal(d, base[kernel]))
                if d is not None:
                    detail = f"{int(np.isfinite(d).sum())} reached, max distance {d[np.isfinite(d)].max():g}"
            results.append({"kernel": kernel, "tool": tool, "valid": ok, "detail": detail})
    if args.tc:
        # No Everygraph kernel: the tools are compared with each other.
        def count(cmd, pattern):
            out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
            return int(re.search(pattern, out).group(1))

        counts = {
            "gap": count([os.path.join(gap_bin, "tc"), "-f", os.path.join(w, f"{g}.sg"), "-n", "1", "-a"],
                         r"(\d+) triangles"),
            "gbbs": count([gbbs_bin, "-s", "-b", "-kernel", "tc", "-rounds", "1", os.path.join(w, f"{g}.adj")],
                          r"### Triangles: (\d+)"),
        }
        if args.python:
            counts["networkit"] = count(py("networkit", "triangles"), r'"triangles": (\d+)')
        for tool in counts:
            results.append({"kernel": "tc", "tool": tool, "valid": len(set(counts.values())) == 1,
                            "detail": f"counts {counts}"})
    if args.leiden:
        results += leiden_checks(csr, args, run, without)
    line = {"graph": g, "threads": args.threads, "source": src, "checked": len(results),
            "invalid": sum(not r["valid"] for r in results), "results": results, "skipped": skipped,
            "skipped_count": len(skipped)}
    if args.out:
        os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
        with open(args.out, "w") as f:
            f.write(json.dumps(line) + "\n")
    print(json.dumps(line), flush=True)


# A Leiden or Louvain run whose modularity is further than this below the best run on the
# graph is broken, not just another local optimum. The working tools of the references
# spec lie up to 0.049 below the best (GVE-Leiden f64 on orkut,
# lab/m3-12-reference-runs/results/validation-orkut.json); a broken run lies far below.
LEIDEN_FLOOR = 0.10


def leiden_checks(csr, args, run, without):
    """One run of each Leiden and Louvain tool of the references spec, its modularity as
    the tool reports it (NetworKit's and igraph's by `modularity`), against the floor."""
    import subprocess

    w, g, t = args.work, csr.name, str(args.threads)
    gve = os.path.join(tool_tree(args, "gve"), "leiden-{}-t" + t)
    gve_q = r"([0-9.]+) modularity[^}]*\} leidenStaticOmp"
    b2 = [args.b2, "run", "--graph", args.graph, "--algo", "leiden", "--threads", t, "--iterations", "2"]
    runs = {
        "lab": (b2, r'"modularity":([0-9.]+)'),
        "lab-det": (b2 + ["--det"], r'"modularity":([0-9.]+)'),
        "gve-f32": ([gve.format("f32"), os.path.join(w, f"{g}.mtx"), "1", "0"], gve_q),
        "gve-f64": ([gve.format("f64"), os.path.join(w, f"{g}.mtx"), "1", "0"], gve_q),
    }
    if args.python:
        for tool, task in (("networkit-leiden", "leiden"), ("networkit-plm", "plm")):
            runs[tool] = (run + ["networkit", task, "--graph", args.graph, "--threads", t], r'"modularity": ([0-9.]+)')
        runs["igraph"] = (run + ["igraph", "leiden", "--graph", args.graph, "--threads", "1", "--edges",
                                 os.path.join(w, f"{g}.edges")], r'"modularity": ([0-9.]+)')
    found = {}
    for tool, (cmd, pattern) in runs.items():
        if tool in without:
            continue
        p = subprocess.run(cmd, capture_output=True, text=True)
        hit = re.search(pattern, p.stdout)
        found[tool] = float(hit.group(1)) if p.returncode == 0 and hit else None
    best = max((q for q in found.values() if q is not None), default=0.0)
    return [{"kernel": "leiden", "tool": tool, "valid": q is not None and q >= best - LEIDEN_FLOOR,
             "detail": f"modularity {q:.4f} (best {best:.4f}, floor {best - LEIDEN_FLOOR:.4f})" if q is not None
             else "no modularity in the output"} for tool, q in found.items()]


def read_sources(args):
    if not args.sources:
        raise SystemExit("bfs64 needs --sources FILE (made by bench/references/gap_sources.cc)")
    return [int(x) for x in open(args.sources).read().split()]


def each_source(run, sources):
    """Runs `run(s)` once per source; returns the mean time and the results."""
    times, results = [], []
    for s in sources:
        t = time.perf_counter()
        results.append(run(s))
        times.append(time.perf_counter() - t)
    return {"mean_seconds": statistics.mean(times), "seconds_all": times, "results": results}


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("tool", choices=sorted(TOOLS) + ["convert", "gap", "gbbs", "pkc", "pasgal",
                                                     "validate"])
    p.add_argument("task", nargs="?", default="")
    p.add_argument("--graph", help="glab-csr-v0 directory")
    p.add_argument("--threads", type=int, default=1)
    p.add_argument("--iterations", type=int, default=20, help="PageRank iterations where the tool takes a count")
    p.add_argument("--work", help="directory for the tool's own files")
    p.add_argument("--input", help="input file for ingest tasks")
    p.add_argument("--memory-limit", default="4GB", help="the tool's own memory limit, where it has one")
    p.add_argument("--edges", help="text edge list (each edge once) for igraph and rustworkx")
    p.add_argument("--out", help="output file for convert")
    p.add_argument("--sources", help="file of source nodes, one per line, for bfs64")
    p.add_argument("--trials", type=int, default=1, help="runs of the call in this process (the median is reported)")
    p.add_argument("--dump", help="write the result here, for validate")
    p.add_argument("--trees", help="the directory of the tools' builds (bench/references/build.sh); default --work")
    p.add_argument("--binary", help="validate: the eg-driver program")
    p.add_argument("--cost", help="validate: the cost-line program")
    p.add_argument("--python", action="store_true", help="validate: NetworKit and igraph too")
    p.add_argument("--tc", action="store_true", help="validate: triangle counts too")
    p.add_argument("--kernels", default="cc,kcore,pr64,bfs,sssp", help="validate: the kernels to check")
    p.add_argument("--without", default="", help="validate: tools not run on this graph, comma-separated")
    p.add_argument("--leiden", action="store_true", help="validate: the modularity floor of the Leiden tools")
    p.add_argument("--b2", help="validate --leiden: the lab Leiden program")
    args = p.parse_args()
    if args.tool == "convert" and args.task == "ldbc-csr":
        if not (args.input and args.out):
            p.error("convert ldbc-csr --input DATASET_DIR --out DIR")
        ldbc_csr(args.input, args.out)
        return
    if not args.graph and args.tool != "duckdb":
        p.error("--graph is required")
    csr = Csr(args.graph, mapped=args.tool in ("convert", "validate")) if args.graph else None
    if args.tool == "convert":
        if not args.out:
            p.error("convert mtx|edges|sg|wsg|wsg-sym|wsg-sym64|gbbs --graph DIR --out FILE")
        convert(csr, args.task, args.out)
        return
    if args.tool in ("gap", "gbbs"):
        (gap if args.tool == "gap" else gbbs)(args.task, csr, args)
        return
    if args.tool in UCR:
        ucr(args.tool, args.task, csr, args)
        return
    if args.tool == "validate":
        validate(csr, args)
        return
    version, load, tasks, storage, dumps = TOOLS[args.tool](args.task, csr, args)
    if args.task not in tasks:
        p.error(f"{args.tool} has no task {args.task!r}; it has: {', '.join(sorted(tasks))}")
    # (algorithm, call) or (algorithm, call, summarize): only the call is timed, --trials
    # times in this process; `seconds` is their median, as for every tool of the
    # references spec.
    algorithm, call, *summarize = tasks[args.task]
    trials = []
    for _ in range(args.trials):
        t = time.perf_counter()
        answer = call()
        trials.append(time.perf_counter() - t)
    seconds = statistics.median(trials)
    if isinstance(answer, dict) and "mean_seconds" in answer:  # one search per source
        seconds = answer.pop("mean_seconds")
    if args.dump:
        if args.task not in dumps:
            p.error(f"{args.tool} {args.task} writes no --dump")
        np.asarray(dumps[args.task](answer)).tofile(args.dump)
    result = summarize[0](answer) if summarize else answer
    print(json.dumps({
        "tool": args.tool, "version": version, "task": args.task, "algorithm": algorithm,
        "options": {"threads": args.threads, "iterations": args.iterations, "storage": storage,
                    "OMP_NUM_THREADS": os.environ.get("OMP_NUM_THREADS"),
                    "affinity": sorted(os.sched_getaffinity(0))},
        "graph": csr.name if csr else args.input, "threads": args.threads,
        "load_seconds": load, "seconds": seconds, "seconds_min": min(trials), "seconds_all": trials,
        "result": result,
    }), flush=True)


if __name__ == "__main__":
    sys.exit(main())
