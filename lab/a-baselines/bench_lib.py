"""Baseline: one tool × one graph per process. Appends JSON lines to results/baselines.jsonl.
usage: bench_lib.py TOOL GRAPH THREADS   (TOOL: networkit | igraph | scipy | rustworkx)"""
import json, os, resource, sys, time

tool, graph, threads = sys.argv[1], sys.argv[2], int(sys.argv[3])
os.environ["OMP_NUM_THREADS"] = str(threads)
import numpy as np

D = f"/home/tux/graphlab/data/csr/{graph}"
meta = json.load(open(f"{D}/meta.json"))
n = meta["n"]
OUT = "/home/tux/graphlab/exp/a-baselines/results/baselines.jsonl"


def rss():
    return round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024)


def rec(task, sec, **kw):
    r = dict(tool=tool, graph=graph, threads=threads, task=task, seconds=round(sec, 3), peak_rss_mb=rss(), **kw)
    print(json.dumps(r), flush=True)
    open(OUT, "a").write(json.dumps(r) + "\n")


def timed(task, fn, **kw):
    t = time.perf_counter()
    out = fn()
    extra = kw.pop("post", lambda o: {})(out)
    rec(task, time.perf_counter() - t, **kw, **extra)
    return out


offsets = np.fromfile(f"{D}/offsets.u64", dtype=np.uint64)
targets = np.fromfile(f"{D}/targets.u32", dtype=np.uint32)
deg = np.diff(offsets).astype(np.int64)
src = np.repeat(np.arange(n, dtype=np.uint32), deg)
hub = int(deg.argmax())
two_m = float(len(targets))


def modularity(labels):
    labels = np.asarray(labels)
    internal = np.count_nonzero(labels[src] == labels[targets])
    vol = np.bincount(labels, weights=deg.astype(float))
    return dict(modularity=round(internal / two_m - float(np.square(vol / two_m).sum()), 6),
                communities=int(len(np.unique(labels))))


if tool == "networkit":
    import networkit as nk
    nk.setNumberOfThreads(threads)
    version = nk.__version__
    up = src < targets
    import scipy.sparse as sp
    G = timed("build_from_arrays", lambda: nk.GraphFromCoo(
        sp.coo_matrix((np.ones(int(up.sum()), dtype=np.float64), (src[up], targets[up])), shape=(n, n)), n=n,
        weighted=False, directed=False))
    assert G.numberOfEdges() == meta["m_undirected"]

    def pr():
        a = nk.centrality.PageRank(G, damp=0.85, tol=1e-30); a.maxIterations = 20; a.run(); return a
    timed("pagerank_20it", pr, post=lambda a: dict(iterations=a.numberOfIterations()))
    timed("components", lambda: nk.components.ConnectedComponents(G).run(),
          post=lambda a: dict(components=a.numberOfComponents()))
    timed("parallel_components", lambda: nk.components.ParallelConnectedComponents(G).run(),
          post=lambda a: dict(components=a.numberOfComponents()))
    timed("kcore", lambda: nk.centrality.CoreDecomposition(G).run(), post=lambda a: dict(max_core=a.maxCoreNumber()))
    timed("bfs_hub", lambda: nk.distance.BFS(G, hub, storePaths=False).run())
    timed("triangles", lambda: nk.centrality.TriangleCount(G).run() if hasattr(nk.centrality, "TriangleCount")
          else nk.centrality.LocalClusteringCoefficient(G).run())
    for name, mk in (("plm", lambda: nk.community.PLM(G, refine=False)),
                     ("plm_refine", lambda: nk.community.PLM(G, refine=True)),
                     ("parallel_leiden", lambda: nk.community.ParallelLeiden(G))):
        timed(name, lambda: mk().run(), post=lambda a: modularity(np.array(a.getPartition().getVector(), dtype=np.int64)))

elif tool == "igraph":
    import igraph as ig
    up = src < targets
    edges = np.column_stack([src[up], targets[up]])
    G = timed("build_from_arrays", lambda: ig.Graph(n=n, edges=edges, directed=False))
    del edges
    timed("pagerank_default", lambda: G.pagerank(damping=0.85))
    timed("components", lambda: G.connected_components(), post=lambda c: dict(components=len(c)))
    timed("kcore", lambda: G.coreness(), post=lambda c: dict(max_core=int(max(c))))
    timed("bfs_hub", lambda: G.bfs(hub))
    if graph == "lj":
        timed("triangles", lambda: G.transitivity_local_undirected(mode="zero"))
    timed("multilevel", lambda: G.community_multilevel(), post=lambda c: modularity(np.array(c.membership)))
    timed("leiden_modularity_2it", lambda: G.community_leiden(objective_function="modularity", n_iterations=2),
          post=lambda c: modularity(np.array(c.membership)))
    timed("leiden_modularity_converge", lambda: G.community_leiden(objective_function="modularity", n_iterations=-1),
          post=lambda c: modularity(np.array(c.membership)))

elif tool == "scipy":
    import scipy.sparse as sp, scipy.sparse.csgraph as cg
    A = timed("build_from_arrays", lambda: sp.csr_matrix(
        (np.ones(len(targets), dtype=np.float32), targets.astype(np.int32), offsets.astype(np.int64)), shape=(n, n)))
    timed("components", lambda: cg.connected_components(A, directed=False), post=lambda c: dict(components=int(c[0])))
    timed("bfs_hub", lambda: cg.breadth_first_order(A, hub, directed=False, return_predecessors=False))

    def pr():
        x = np.full(n, 1.0 / n); inv = np.where(deg > 0, 1.0 / np.maximum(deg, 1), 0.0)
        for _ in range(20):
            x = 0.15 / n + 0.85 * (A @ (x * inv))
        return x
    timed("pagerank_20it", pr)

elif tool == "rustworkx":
    import rustworkx as rx
    up = src < targets

    def build():
        g = rx.PyGraph(multigraph=False); g.add_nodes_from(range(n))
        g.extend_from_edge_list(list(zip(src[up].tolist(), targets[up].tolist()))); return g
    G = timed("build_from_arrays", build)
    timed("components", lambda: rx.connected_components(G), post=lambda c: dict(components=len(c)))
    timed("kcore", lambda: rx.core_number(G))
    timed("bfs_hub", lambda: rx.bfs_successors(G, hub))
