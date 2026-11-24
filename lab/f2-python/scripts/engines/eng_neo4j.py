# Assumes the Neo4j server (started by run_all.sh, bound to 127.0.0.1:17687) holds the imported lj graph.
import time, sys, json, neo4j
from neo4j import GraphDatabase, Query
from common import *
name = sys.argv[1] if len(sys.argv) > 1 else "lj"
import_s = float(sys.argv[2]) if len(sys.argv) > 2 else None
disk_mb = float(sys.argv[3]) if len(sys.argv) > 3 else None
pid = int(sys.argv[4]) if len(sys.argv) > 4 else None
q = queries(name); T = truth(name)
drv = GraphDatabase.driver("bolt://127.0.0.1:17687", auth=None)
s = drv.session()
t0 = time.perf_counter()
s.run("CREATE INDEX nid IF NOT EXISTS FOR (n:N) ON (n.id)").consume()
s.run("CALL db.awaitIndexes(600)").consume()
idx_s = time.perf_counter() - t0
ver = s.run("CALL dbms.components() YIELD versions RETURN versions[0]").single()[0]
r = {"engine": "Neo4j Community", "version": ver, "graph": name, "load_s": import_s, "index_s": idx_s, "disk_mb": disk_mb,
     "storage": "neo4j-admin import (offline, 3 threads), each edge once, undirected pattern (a)--(b); heap 3g pagecache 1g; queries over bolt (client-server, localhost)",
     "threads": 3}
def mk(cy, to=10.0):
    def f(params):
        try:
            return s.run(Query(cy, timeout=to), params).values()
        except neo4j.exceptions.ClientError as e:
            if "Timeout" in str(e.code) or "Terminated" in str(e.code) or "timeout" in str(e).lower(): raise TimeoutError()
            raise
    return f
nbq = mk("MATCH (a:N {id:$x})--(b) RETURN b.id")
st, nb = bench(lambda v: len(nbq({"x": v})), q["nodes"]); r["neighbors"] = st
k2 = mk("MATCH (a:N {id:$x})-[*1..2]-(b) WHERE b <> a RETURN count(DISTINCT b)")
st, h2 = bench(lambda v: k2({"x": v})[0][0], q["nodes"]); r["hop2"] = st
k3 = mk("MATCH (a:N {id:$x})-[*1..3]-(b) WHERE b <> a RETURN count(DISTINCT b)")
st, h3 = bench(lambda v: k3({"x": v})[0][0], q["nodes3"]); r["hop3"] = st
spq = mk("MATCH (a:N {id:$x}), (b:N {id:$y}) MATCH p = shortestPath((a)-[*..30]-(b)) RETURN length(p)")
def sp(p):
    if p[0] == p[1]: return 0
    o = spq({"x": p[0], "y": p[1]}); return o[0][0] if o else -1
st, sps = bench(sp, q["pairs"]); r["sp"] = st
def timed(cy):
    t = time.perf_counter(); o = s.run(cy).values(); return time.perf_counter() - t, o
try:
    s.run("CALL gds.graph.drop('g', false)").consume()
    r["gds_project_s"], _ = timed("CALL gds.graph.project('g', 'N', {E: {orientation: 'UNDIRECTED'}}) YIELD relationshipCount RETURN relationshipCount")
    r["pagerank_s"], o = timed("CALL gds.pageRank.stats('g', {maxIterations: 20, tolerance: 0.0, concurrency: 3}) YIELD ranIterations RETURN ranIterations")
    r["pagerank_note"] = f"GDS 2.13.2 pageRank.stats, 20 iterations fixed, concurrency 3 (excludes projection), iters={o}"
    r["wcc_s"], o = timed("CALL gds.wcc.stats('g', {concurrency: 3}) YIELD componentCount RETURN componentCount")
    r["n_components"] = o[0][0]
except Exception as e:
    r["gds_err"] = str(e)[:300]
if pid:
    for line in open(f"/proc/{pid}/status"):
        if line.startswith("VmHWM"): r["server_peak_rss_mb"] = int(line.split()[1]) / 1024
if T:
    r["check"] = {k: check(o, T[k]) for k, o in [("neighbors", nb), ("hop2", h2), ("hop3", h3), ("sp", sps)]}
write(f"neo4j_{name}", r)
