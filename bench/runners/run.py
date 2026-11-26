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

Also `python run.py convert mtx --graph DIR --out FILE`: MatrixMarket for GVE; `sg` and
`wsg`: GAP's serialized graphs, the second with the costs of `edge_map_bench --kernel
sssp` (integers 1 to 255, a function of the entry); `ldbc-csr --input DATASET --out DIR`:
a weighted LDBC data set as a CSR directory with its costs; `gbbs`: GBBS's binary
symmetric format. And `python run.py gap|gbbs TASK --graph DIR --work DIR`: the GAP
Benchmark Suite or GBBS binaries built in `--work`, on the graph files made there
(`GAP_TASKS`, `GBBS_TASKS`); `seconds` is the tool's own kernel time (best trial, or the
mean over the sources of `--sources`). `bfs64` of NetworKit and igraph also runs one BFS
per source of `--sources` and reports the mean.
"""

import argparse
import json
import os
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
        # Mapped for the converters, which stream it; read into the heap for the tools.
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
        # Per-edge triangle counts; each triangle is on three edges. Timed: the edge index
        # the algorithm requires, and run(). Not timed: scores(), which copies 117 M floats
        # into a Python list on orkut.
        "triangles": ("networkit.Graph.indexEdges() + networkit.sparsification.TriangleEdgeScore.run(), "
                      "scores summed / 3 after the clock (an exact triangle count)",
                      lambda: (g.indexEdges(), run_then(nk.sparsification.TriangleEdgeScore(g)))[1],
                      lambda a: {"triangles": int(sum(a.scores()) // 3)}),
        "plm": ("networkit.community.PLM(refine=False) (parallel Louvain)",
                *partition(lambda: nk.community.PLM(g, refine=False))),
        "leiden": ("networkit.community.ParallelLeiden (defaults)",
                   *partition(lambda: nk.community.ParallelLeiden(g))),
    }
    return nk.__version__, load, tasks, "networkit.GraphFromCoo, undirected, unweighted, from the CSR"


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
        # No triangles task: igraph's Python API lists them (a tuple each, 177 M on lj)
        # rather than counting, and transitivity is a different quantity.
        "multilevel": ("igraph.Graph.community_multilevel (Louvain)", lambda: g.community_multilevel(),
                       lambda c: modularity(csr, c.membership)),
        "leiden": ("igraph.Graph.community_leiden(objective_function='modularity', n_iterations=2)",
                   lambda: g.community_leiden(objective_function="modularity", n_iterations=2),
                   lambda c: modularity(csr, c.membership)),
        "queries": ("neighbors(), neighborhood_size(order=2), get_shortest_path() on the lab's seed-42 query set", queries),
    }
    return ig.__version__, load, tasks, "igraph.Graph.Read_Edgelist(text, each edge once), undirected"


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
    return rx.__version__, load, tasks, "rustworkx.PyGraph.read_edge_list(text, each edge once), undirected"


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
    return ddb.__version__, 0.0, tasks, f"text edge list {args.input}; memory_limit {args.memory_limit}"


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
    return version, load, tasks, "polars edge frame, each edge once, from the CSR"


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
    return version, load, tasks, "on-disk node and rel tables (each edge once), COPY FROM Parquet; load = COPY + CHECKPOINT"


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


CHUNK = 1 << 26  # entries per write: the converters stream Friendster's 3.6 G entries


def write_targets(csr, f, weighted=False):
    m = len(csr.targets)
    with np.errstate(over="ignore"):
        for lo in range(0, m, CHUNK):
            t = np.asarray(csr.targets[lo:lo + CHUNK])
            if weighted:
                pairs = np.empty(2 * len(t), dtype=np.int32)
                pairs[0::2] = t.view(np.int32)
                pairs[1::2] = uniform_costs(len(t), lo)
                pairs.tofile(f)
            else:
                t.tofile(f)


def gap_serialized(csr, weighted, out):
    """GAP's .sg / .wsg (src/reader.h, ReadSerializedGraph), undirected: a bool, the
    number of entries and of nodes as int64, the int64 offsets, then the int32 targets,
    or (target, weight) pairs of int32."""
    m = len(csr.targets)
    tmp = out + ".part"
    with open(tmp, "wb") as f:
        f.write(np.array([0], dtype=np.uint8).tobytes())
        f.write(np.array([m, csr.n], dtype=np.int64).tobytes())
        csr.offsets.astype(np.int64).tofile(f)
        write_targets(csr, f, weighted)
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
    (for igraph and rustworkx, as lab/f2-python). `sg`, `wsg`: GAP's (above)."""
    if kind in ("sg", "wsg"):
        os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
        gap_serialized(csr, kind == "wsg", out)
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
        raise SystemExit(f"convert {kind}: use mtx, edges, sg, wsg or gbbs")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    tmp = out + ".part"
    with open(tmp, "wb") as f:
        f.write(header.encode())
        pc.write_csv(pa.table(cols), f, pc.WriteOptions(include_header=False, delimiter=" "))
    os.replace(tmp, out)


# GAP tasks: (binary, graph kind, arguments, how the trial times reduce, algorithm).
# `bfs` and `sssp` start from the hub (bench/specs/kernels.toml); `bfs64` and `sssp64`
# from GAP's own 64 seeded non-zero-degree sources (no -r: SourcePicker, the list that
# bench/references/gap_sources.cc prints), one trial each, reported as their mean.
# `pr` and `pr-gs` are GAP's pr_spmv.cc (Jacobi) and pr.cc (Gauss-Seidel) with ScoreT
# changed from float to double (the references spec builds them), 20 iterations, tol 0.
GAP_TASKS = {
    "bfs": ("bfs", "sg", lambda c: ["-r", str(c.hub()), "-n", "5"], min, "bfs.cc direction-optimizing, from the hub"),
    "sssp": ("sssp", "wsg", lambda c: ["-r", str(c.hub()), "-n", "5", "-d", "2"], min,
             "sssp.cc delta-stepping, -d 2, from the hub"),
    "bfs64": ("bfs", "sg", lambda c: ["-n", "64"], statistics.mean,
              "bfs.cc direction-optimizing, 64 sources of SourcePicker, mean per source"),
    "sssp64": ("sssp", "wsg", lambda c: ["-n", "64", "-d", "2"], statistics.mean,
               "sssp.cc delta-stepping -d 2, int32 costs 1..255, 64 sources of SourcePicker, mean per source"),
    "cc": ("cc", "sg", lambda c: ["-n", "5"], min, "cc.cc Afforest (neighbour rounds 2, sampled skip)"),
    "pr": ("pr_spmv64", "sg", lambda c: ["-n", "5", "-i", "20", "-t", "0"], min,
           "pr_spmv.cc Jacobi pull, ScoreT=double, 20 iterations, tol 0"),
    "pr-gs": ("pr64", "sg", lambda c: ["-n", "5", "-i", "20", "-t", "0"], min,
              "pr.cc Gauss-Seidel pull, ScoreT=double, 20 iterations, tol 0"),
    "tc": ("tc", "sg", lambda c: ["-n", "3"], min, "tc.cc ordered count, relabels by degree when its heuristic says so"),
}


def gap(task, csr, args):
    """GAP's own binaries (SSSP: Δ = 2, its best on these costs among 2, 8 and 32 in an
    exploratory sweep), parsed: its trial times, not the process."""
    import subprocess

    binary, kind, extra, reduce, algorithm = GAP_TASKS[task]
    graph = os.path.join(args.work, f"{csr.name}.{kind}")
    cmd = [os.path.join(args.work, "gapbs", binary), "-f", graph, *extra(csr)]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    trials = [float(line.split()[-1]) for line in out.splitlines() if line.startswith("Trial Time")]
    load = sum(float(line.split()[-1]) for line in out.splitlines() if line.startswith(("Read Time", "Build Time")))
    version = os.path.join(args.work, "gapbs", "COMMIT")
    commit = open(version).read().strip() if os.path.exists(version) else subprocess.run(
        ["git", "-C", os.path.join(args.work, "gapbs"), "rev-parse", "HEAD"],
        check=True, capture_output=True, text=True).stdout.strip()
    print(json.dumps({
        "tool": "gap", "version": commit, "task": task, "algorithm": algorithm,
        "options": {"OMP_NUM_THREADS": os.environ.get("OMP_NUM_THREADS"), "command": cmd[1:]},
        "graph": csr.name, "threads": args.threads, "load_seconds": load, "seconds": reduce(trials), "trials": len(trials),
        "seconds_all": trials,
    }))


# GBBS tasks: (binary under the tree, arguments, how the round times reduce, algorithm).
GBBS_TASKS = {
    "bfs64": ("benchmarks/BFS/NonDeterministicBFS/BFS_sources", lambda a: ["-rounds", "1", "-sources", a.sources],
              "NonDeterministicBFS BFS(G, src) from each listed source (bench/references/gbbs_bfs_sources.cc), "
              "mean per source"),
    "cc": ("benchmarks/Connectivity/WorkEfficientSDB14/Connectivity", lambda a: ["-rounds", "5"],
           "WorkEfficientSDB14 connectivity (beta 0.2)"),
    "pr": ("benchmarks/PageRank/PageRank", lambda a: ["-rounds", "5", "-iters", "20", "-eps", "0"],
           "PageRank (dense edgeMapReduce, double), 20 iterations, eps 0"),
    "kcore": ("benchmarks/KCore/JulienneDBS17/KCore", lambda a: ["-rounds", "5"],
              "JulienneDBS17 KCore (bucketing, 16 buckets)"),
    "tc": ("benchmarks/TriangleCounting/ShunTangwongsan15/Triangle", lambda a: ["-rounds", "3"],
           "ShunTangwongsan15 Triangle (degree ordering)"),
}


def gbbs(task, csr, args):
    """GBBS's own binaries on its binary symmetric format (`convert gbbs`), memory-mapped
    (-b reads the file in place), threads from PARLAY_NUM_THREADS. `seconds` is GBBS's
    "### Running Time" (the kernel alone), the best round, or the mean over sources."""
    import subprocess

    binary, extra, algorithm = GBBS_TASKS[task]
    if task == "bfs64" and not args.sources:
        raise SystemExit("gbbs bfs64 needs --sources FILE")
    cmd = [os.path.join(args.work, "gbbs", binary), "-s", "-b", *extra(args), os.path.join(args.work, f"{csr.name}.adj")]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    rounds = [float(line.split()[-1]) for line in out.splitlines() if line.startswith("### Running Time")]
    per_source = [float(line.split()[-1]) for line in out.splitlines() if line.startswith("### Source Time")]
    print(json.dumps({
        "tool": "gbbs", "version": open(os.path.join(args.work, "gbbs", "COMMIT")).read().strip(), "task": task,
        "algorithm": algorithm,
        "options": {"PARLAY_NUM_THREADS": os.environ.get("PARLAY_NUM_THREADS"), "command": cmd[1:]},
        "graph": csr.name, "threads": args.threads, "seconds": min(rounds), "seconds_all": per_source or rounds,
    }))


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
    p.add_argument("tool", choices=sorted(TOOLS) + ["convert", "gap", "gbbs"])
    p.add_argument("task")
    p.add_argument("--graph", help="glab-csr-v0 directory")
    p.add_argument("--threads", type=int, default=1)
    p.add_argument("--iterations", type=int, default=20, help="PageRank iterations where the tool takes a count")
    p.add_argument("--work", help="directory for the tool's own files")
    p.add_argument("--input", help="input file for ingest tasks")
    p.add_argument("--memory-limit", default="4GB", help="the tool's own memory limit, where it has one")
    p.add_argument("--edges", help="text edge list (each edge once) for igraph and rustworkx")
    p.add_argument("--out", help="output file for convert")
    p.add_argument("--sources", help="file of source nodes, one per line, for bfs64")
    args = p.parse_args()
    if args.tool == "convert" and args.task == "ldbc-csr":
        if not (args.input and args.out):
            p.error("convert ldbc-csr --input DATASET_DIR --out DIR")
        ldbc_csr(args.input, args.out)
        return
    if not args.graph and args.tool != "duckdb":
        p.error("--graph is required")
    csr = Csr(args.graph, mapped=args.tool == "convert") if args.graph else None
    if args.tool == "convert":
        if not args.out:
            p.error("convert mtx|edges|sg|wsg|gbbs --graph DIR --out FILE")
        convert(csr, args.task, args.out)
        return
    if args.tool in ("gap", "gbbs"):
        (gap if args.tool == "gap" else gbbs)(args.task, csr, args)
        return
    version, load, tasks, storage = TOOLS[args.tool](args.task, csr, args)
    if args.task not in tasks:
        p.error(f"{args.tool} has no task {args.task!r}; it has: {', '.join(sorted(tasks))}")
    # (algorithm, call) or (algorithm, call, summarize): only the call is timed.
    algorithm, call, *summarize = tasks[args.task]
    t = time.perf_counter()
    answer = call()
    seconds = time.perf_counter() - t
    if isinstance(answer, dict) and "mean_seconds" in answer:  # one search per source
        seconds = answer.pop("mean_seconds")
    result = summarize[0](answer) if summarize else answer
    print(json.dumps({
        "tool": args.tool, "version": version, "task": args.task, "algorithm": algorithm,
        "options": {"threads": args.threads, "iterations": args.iterations, "storage": storage,
                    "OMP_NUM_THREADS": os.environ.get("OMP_NUM_THREADS"),
                    "affinity": sorted(os.sched_getaffinity(0))},
        "graph": csr.name if csr else args.input, "threads": args.threads,
        "load_seconds": load, "seconds": seconds, "result": result,
    }), flush=True)


if __name__ == "__main__":
    sys.exit(main())
