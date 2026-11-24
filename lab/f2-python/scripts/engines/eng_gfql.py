import time, sys, graphistry, polars as pl
from graphistry import n, e_undirected
from common import *
name = sys.argv[1] if len(sys.argv) > 1 else "lj"
q = queries(name); T = truth(name)
t0 = time.perf_counter()
E = pl.read_parquet(f"{DATA}/{name}/edges_und.parquet"); N = pl.read_parquet(f"{DATA}/{name}/nodes.parquet")
g = graphistry.edges(E, "src", "dst").nodes(N, "id")
load = time.perf_counter() - t0
r = {"engine": "GFQL (pygraphistry)", "version": f"graphistry {graphistry.__version__}, polars {pl.__version__}",
     "graph": name, "load_s": load, "disk_mb": None, "threads": 3,
     "storage": "in-memory polars edge frame, each edge once, e_undirected(); engine='polars'"}
K = 10 if QUICK else 30
def hop(v, k):
    res = g.gfql([n({"id": v}), e_undirected(hops=k), n()], engine="polars")
    return len(res._nodes) - 1
st, nb = bench(lambda v: hop(v, 1), q["nodes"][:K], warm=1); r["neighbors"] = st
st, h2 = bench(lambda v: hop(v, 2), q["nodes"][:K], warm=1); r["hop2"] = st
st, h3 = bench(lambda v: hop(v, 3), q["nodes3"][:K], warm=1); r["hop3"] = st
r["sp"] = None; r["sp_note"] = "GFQL has no shortest-path operator (would need a client-side BFS loop of hop() calls)"
r["inputs_note"] = f"first {K} inputs per query type (each query is a full scan of the edge frame per hop)"
t = time.perf_counter()
nd = g.get_degrees()._nodes
r["degree_frame_type"] = type(nd).__module__  # get_degrees() returns a pandas frame even for polars input
dd = nd.groupby("degree").size() if hasattr(nd, "groupby") else nd.group_by("degree").len()
r["degree_dist_s"] = time.perf_counter() - t
r["pagerank_s"] = None; r["pagerank_note"] = "PageRank only via compute_igraph/compute_cugraph plugins (not GFQL-native); not run"
if T:
    r["check"] = {k: check(o, T[k][:len(o)]) for k, o in [("hop2", h2), ("hop3", h3)]}
    r["check"]["neighbors"] = check(nb, T["neighbors"][:len(nb)])
write(f"gfql_{name}", r)
