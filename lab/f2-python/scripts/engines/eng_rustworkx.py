import time, sys, numpy as np, rustworkx as rx
from common import *
name = sys.argv[1] if len(sys.argv) > 1 else "lj"
q = queries(name); n = q["n"]; T = truth(name)
t0 = time.perf_counter()
G = rx.PyGraph.read_edge_list(f"{DATA}/{name}/edges_und.txt", deliminator=" ")
while len(G) < n: G.add_node(None)
load = time.perf_counter() - t0
r = {"engine": "rustworkx", "version": rx.__version__, "graph": name, "load_s": load, "disk_mb": None,
     "storage": "in-memory petgraph StableGraph, undirected, each edge once; read_edge_list from text"}
st, nb = bench(lambda v: len(G.neighbors(v)), q["nodes"]); r["neighbors"] = st
def khop(v, k):  # no native k-hop count for PyGraph: Python BFS over G.neighbors()
    seen = {v}; fr = [v]
    for _ in range(k):
        nxt = []
        for u in fr:
            for w in G.neighbors(u):
                if w not in seen: seen.add(w); nxt.append(w)
        fr = nxt
    return len(seen) - 1
st, h2 = bench(lambda v: khop(v, 2), q["nodes"]); r["hop2"] = st; r["hop2_note"] = "python BFS loop over native neighbors()"
st, h3 = bench(lambda v: khop(v, 3), q["nodes3"]); r["hop3"] = st
def sp(p):
    d = rx.dijkstra_shortest_path_lengths if False else None
    paths = rx.graph_dijkstra_shortest_paths(G, p[0], target=p[1], default_weight=1.0)
    return len(paths[p[1]]) - 1 if p[1] in paths else -1
st, sps = bench(sp, q["pairs"]); r["sp"] = st; r["sp_note"] = "native Dijkstra(weight=1) with target early-exit"
t = time.perf_counter(); cc = rx.connected_components(G); r["wcc_s"] = time.perf_counter() - t
r["n_components"] = len(cc)
r["pagerank_s"] = None; r["pagerank_note"] = "rustworkx.pagerank is PyDiGraph-only; not run"
if T:
    r["check"] = {k: check(o, T[k]) for k, o in [("neighbors", nb), ("hop2", h2), ("hop3", h3), ("sp", sps)]}
write(f"rustworkx_{name}", r)
