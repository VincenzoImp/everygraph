# Runs under .venv-duck (duckdb 1.3.2): duckpgq's parser hook does not work on DuckDB 2.0.1, and its
# graph algorithm table functions fail on 1.4.4 ("csr_cte does not exist").
import time, sys, os, threading, duckdb
from common import *
name = sys.argv[1] if len(sys.argv) > 1 else "lj"
q = queries(name); n = q["n"]; T = truth(name)
path = f"{ROOT}/db/duckdb_{name}.duckdb"
for p in (path, path + ".wal"):
    if os.path.exists(p): os.remove(p)
c = duckdb.connect(path, config={"extension_directory": f"{ROOT}/dl/home/.duckdb/extensions",
                                 "threads": 3, "memory_limit": "4GB"})
c.execute("INSTALL duckpgq FROM community"); c.execute("LOAD duckpgq")
t0 = time.perf_counter()
c.execute(f"CREATE TABLE nodes AS SELECT id FROM '{DATA}/{name}/nodes.parquet'")
c.execute(f"CREATE TABLE edges AS SELECT src, dst FROM '{DATA}/{name}/edges_both.parquet'")  # sorted by src
c.execute("CREATE PROPERTY GRAPH g VERTEX TABLES (nodes) EDGE TABLES (edges SOURCE KEY (src) REFERENCES nodes (id) "
          "DESTINATION KEY (dst) REFERENCES nodes (id))")
c.execute("CHECKPOINT")
load = time.perf_counter() - t0
print("loaded", load, flush=True)
r = {"engine": "DuckDB+DuckPGQ", "version": f"duckdb {duckdb.__version__}, duckpgq community", "graph": name,
     "load_s": load, "disk_mb": du_mb(path), "threads": 3,
     "storage": "on-disk DuckDB tables; edges stored both directions (69.4M rows, sorted by src); PGQ patterns directed ->"}
TO = 10.0
KP = 2 if QUICK else 5  # PGQ path queries are slow (each ->{1,2} timed out at 10 s in 30/30 tries; CSR rebuilt per query)
def prog(tag):
    print("STAGE", tag, round(time.perf_counter() - t0, 1), "s, rss_mb", round(peak_rss_mb()), flush=True)
    write(f"duckpgq_{name}", dict(r, partial=True, last_stage=tag))
def run(sql, params=()):
    tm = threading.Timer(TO, c.interrupt); tm.start()
    try:
        return c.execute(sql, params).fetchall()
    except duckdb.InterruptException:
        raise TimeoutError()
    finally:
        tm.cancel()
st, nb = bench(lambda v: len(run("SELECT dst FROM edges WHERE src = ?", [v])), q["nodes"], timeout_s=TO); r["neighbors_sql"] = st
prog("neighbors_sql")
st, nbp = bench(lambda v: run(f"SELECT count(*) FROM GRAPH_TABLE (g MATCH (a:nodes WHERE a.id = {int(v)})-[e:edges]->(b:nodes) COLUMNS (b.id AS bid))")[0][0], q["nodes"][:KP], timeout_s=TO, warm=1)
r["neighbors"] = st; r["neighbors_note"] = "PGQ MATCH (first KP inputs); neighbors_sql = plain SQL point lookup"; prog("neighbors")
st, h2 = bench(lambda v: run("SELECT count(DISTINCT dst) FROM (SELECT e2.dst FROM edges e1 JOIN edges e2 ON e2.src = e1.dst WHERE e1.src = $1 UNION ALL SELECT dst FROM edges WHERE src = $1) WHERE dst <> $1", [v]), q["nodes"], timeout_s=TO)
h2 = [x[0][0] if isinstance(x, list) else x for x in h2]; r["hop2_sql"] = st; prog("hop2_sql")
st, h2p = bench(lambda v: run(f"SELECT count(DISTINCT bid) FROM GRAPH_TABLE (g MATCH (a:nodes WHERE a.id = {int(v)})-[e:edges]->{{1,2}}(b:nodes) COLUMNS (b.id AS bid)) WHERE bid <> {int(v)}")[0][0], q["nodes"][:KP], timeout_s=TO, warm=1)
r["hop2"] = st; r["hop2_note"] = "PGQ ->{1,2} (first KP inputs); hop2_sql = 2 self-joins"; prog("hop2")
st, h3 = bench(lambda v: run(f"SELECT count(DISTINCT bid) FROM GRAPH_TABLE (g MATCH p = ANY SHORTEST (a:nodes WHERE a.id = {int(v)})-[e:edges]->{{1,3}}(b:nodes) COLUMNS (b.id AS bid)) WHERE bid <> {int(v)}")[0][0], q["nodes3"][:KP], timeout_s=TO, warm=1)
r["hop3"] = st; r["hop3_note"] = "PGQ ANY SHORTEST ->{1,3} (first KP inputs)"; prog("hop3")
st, h3s = bench(lambda v: run("""WITH l1 AS (SELECT DISTINCT dst FROM edges WHERE src = $1),
 l2 AS (SELECT DISTINCT e.dst FROM edges e JOIN l1 ON e.src = l1.dst),
 l3 AS (SELECT DISTINCT e.dst FROM edges e JOIN l2 ON e.src = l2.dst)
 SELECT count(DISTINCT dst) FROM (FROM l1 UNION ALL FROM l2 UNION ALL FROM l3) WHERE dst <> $1""", [v])[0][0], q["nodes3"], timeout_s=TO)
r["hop3_sql"] = st; prog("hop3_sql")
TO = 600.0
for nm, sql in [("pagerank", "SELECT count(*), max(pagerank) FROM pagerank(g, nodes, edges)"),
                ("degree_dist", "SELECT d, count(*) FROM (SELECT src, count(*) d FROM edges GROUP BY src) GROUP BY d"),
                ] + ([("wcc", "SELECT count(DISTINCT componentId) FROM weakly_connected_component(g, nodes, edges)")]
                     if os.environ.get("DUCKPGQ_OOM_STAGES", "0") == "1" else []):  # wcc OOM-killed at 6 GB on lj
    t = time.perf_counter()
    try:
        prog("before_" + nm); o = run(sql); r[f"{nm}_s"] = time.perf_counter() - t; r[f"{nm}_out"] = o[:3]
    except Exception as e:
        r[f"{nm}_s"] = None; r[f"{nm}_err"] = str(e)[:300]
OOM_STAGES = os.environ.get("DUCKPGQ_OOM_STAGES", "0") == "1"
if OOM_STAGES:
    TO = 10.0
    def sp(p):
        if p[0] == p[1]: return 0
        o = run(f"FROM GRAPH_TABLE (g MATCH p = ANY SHORTEST (a:nodes WHERE a.id = {int(p[0])})-[e:edges]->*(b:nodes WHERE b.id = {int(p[1])}) COLUMNS (path_length(p) AS len))")
        return o[0][0] if o else -1
    st, sps = bench(sp, q["pairs"][:KP], timeout_s=TO, warm=1); r["sp"] = st; r["sp_note"] = "first KP pairs"
    prog("sp")
else:
    r["sp"] = {"n": 0, "oom": True}; r["sp_note"] = "ANY SHORTEST ->* point-to-point was OOM-killed (>6 GB cgroup) on the first lj pair; skipped (DUCKPGQ_OOM_STAGES=1 to retry)"
if not os.environ.get("DUCKPGQ_OOM_STAGES") == "1":
    r["wcc_s"] = None; r["wcc_err"] = "weakly_connected_component OOM-killed (>6 GB cgroup) on lj; skipped"
if T:
    r["check"] = {lab: check(o, T[k][:len(o)]) for lab, k, o in [("neighbors_sql", "neighbors", nb), ("neighbors", "neighbors", nbp),
        ("hop2_sql", "hop2", h2), ("hop2", "hop2", h2p), ("hop3", "hop3", h3), ("hop3_sql", "hop3", h3s)] + ([("sp", "sp", sps)] if OOM_STAGES else [])}
write(f"duckpgq_{name}", r)
