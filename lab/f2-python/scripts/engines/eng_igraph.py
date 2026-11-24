import os, time, json, sys, numpy as np, pyarrow.parquet as pq, igraph as ig
from common import *
name = sys.argv[1] if len(sys.argv) > 1 else "lj"
q = queries(name); n = q["n"]
t0 = time.perf_counter()
G = ig.Graph.Read_Edgelist(f"{DATA}/{name}/edges_und.txt", directed=False)  # C reader, "src dst" lines
G.add_vertices(n - G.vcount())
load = time.perf_counter() - t0
r = {"engine": "python-igraph", "version": ig.__version__, "graph": name, "load_s": load,
     "storage": "in-memory, undirected edge list (each edge once); loaded via Read_Edgelist from text (numpy 2D constructor OOMs >6GB)", "disk_mb": None}
st, nb = bench(lambda v: len(G.neighbors(v)), q["nodes"]); r["neighbors"] = st
st, h2 = bench(lambda v: G.neighborhood_size(v, order=2) - 1, q["nodes"]); r["hop2"] = st
st, h3 = bench(lambda v: G.neighborhood_size(v, order=3) - 1, q["nodes3"]); r["hop3"] = st
def sp(p):  # get_shortest_path = BFS with early exit at target (distances() does a full BFS)
    path = G.get_shortest_path(p[0], p[1])
    return len(path) - 1 if path else -1
st, sps = bench(sp, q["pairs"]); r["sp"] = st
t = time.perf_counter(); pr = G.pagerank(); r["pagerank_s"] = time.perf_counter() - t
t = time.perf_counter(); cc = G.connected_components(); r["wcc_s"] = time.perf_counter() - t
r["n_components"] = len(cc)
t = time.perf_counter(); dd = np.bincount(G.degree()); r["degree_dist_s"] = time.perf_counter() - t
if not QUICK or not os.path.exists(f"{RES}/truth_{name}.json"):
    json.dump({"neighbors": nb, "hop2": h2, "hop3": h3, "sp": sps, "n_components": len(cc),
               "pr_top": int(np.argmax(pr)), "pr_max": max(pr)}, open(f"{RES}/truth_{name}.json", "w"))
sp_t = r["sp"]
st, _ = bench(lambda p: G.distances(p[0], p[1])[0][0], q["pairs"][:10]); r["sp_full_bfs"] = st
write(f"igraph_{name}", r)
