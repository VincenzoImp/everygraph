import time, sys, os, shutil, ladybug as lb
from common import *
name = sys.argv[1] if len(sys.argv) > 1 else "lj"
q = queries(name); n = q["n"]; T = truth(name)
path = f"{ROOT}/db/ladybug_{name}"
if os.path.exists(path): shutil.rmtree(path) if os.path.isdir(path) else os.remove(path)
for ext in (".wal", ".shadow"):
    if os.path.exists(path + ext): os.remove(path + ext)
db = lb.Database(path, buffer_pool_size=3 << 30, max_num_threads=3)
c = lb.Connection(db, num_threads=3)
t0 = time.perf_counter()
c.execute("CREATE NODE TABLE N(id INT64 PRIMARY KEY)")
c.execute("CREATE REL TABLE E(FROM N TO N)")
c.execute(f"COPY N FROM '{DATA}/{name}/nodes.parquet'")
c.execute(f"COPY E FROM '{DATA}/{name}/edges_und.parquet'")
load = time.perf_counter() - t0
c.execute("CHECKPOINT")
r = {"engine": "LadybugDB", "version": lb.__version__ if hasattr(lb, "__version__") else "0.22.3", "graph": name,
     "load_s": load, "disk_mb": sum(du_mb(p) or 0 for p in [path, path + ".wal"]),
     "storage": "on-disk, rel table E stored once (src<dst), queried with undirected pattern -[:E]-; COPY FROM parquet",
     "threads": 3}
c.set_query_timeout(10000)
def one(stmt):
    ps = c.prepare(stmt)
    def f(params):
        try:
            res = c.execute(ps, params)
        except RuntimeError as e:
            if "interrupt" in str(e).lower() or "timeout" in str(e).lower(): raise TimeoutError()
            raise
        return res.get_all()
    return f
nbq = one("MATCH (a:N {id:$x})-[:E]-(b:N) RETURN b.id")
st, nb = bench(lambda v: len(nbq({"x": v})), q["nodes"]); r["neighbors"] = st
k2 = one("MATCH (a:N {id:$x})-[:E* SHORTEST 1..2]-(b:N) RETURN count(DISTINCT b.id)")
st, h2 = bench(lambda v: k2({"x": v})[0][0], q["nodes"]); r["hop2"] = st
r["hop2_note"] = "SHORTEST 1..2 (BFS semantics); WALK variant below"
k2w = one("MATCH (a:N {id:$x})-[:E*1..2]-(b:N) WHERE b.id <> $x RETURN count(DISTINCT b.id)")
st, h2w = bench(lambda v: k2w({"x": v})[0][0], q["nodes"]); r["hop2_walk"] = st
k3 = one("MATCH (a:N {id:$x})-[:E* SHORTEST 1..3]-(b:N) RETURN count(DISTINCT b.id)")
st, h3 = bench(lambda v: k3({"x": v})[0][0], q["nodes3"]); r["hop3"] = st
spq = one("MATCH p=(a:N {id:$x})-[:E* SHORTEST 1..30]-(b:N {id:$y}) RETURN length(p)")
def sp(p):
    if p[0] == p[1]: return 0
    o = spq({"x": p[0], "y": p[1]}); return o[0][0] if o else -1
st, sps = bench(sp, q["pairs"]); r["sp"] = st
c.set_query_timeout(600000)
t = time.perf_counter()
dd = c.execute("MATCH (a:N)-[:E]-(b:N) WITH a, count(*) AS d RETURN d, count(*) AS c ORDER BY d").get_all()
r["degree_dist_s"] = time.perf_counter() - t
r["degree_dist_nodes"] = sum(x[1] for x in dd)
r["pagerank_s"] = None
r["pagerank_note"] = ("algo extension (page_rank/wcc) installs but fails to load: needs libnetworkit.so from a "
                      "patched NetworKit (undefined symbol NetworKit::ParallelCoreDecomposition with pip networkit 11.2/11.3)")
if T:
    r["check"] = {lab: check(o, T[k]) for lab, k, o in [("neighbors","neighbors", nb), ("hop2","hop2", h2), ("hop2_walk","hop2", h2w), ("hop3","hop3", h3), ("sp","sp", sps)]}
write(f"ladybug_{name}", r)
