"""DuckDB ingest bar: CSV edge list -> dense ids -> symmetric, deduplicated, sorted (src,dst) Parquet.
usage: duck_ingest.py GRAPH THREADS"""
import duckdb, json, sys, time, resource, os, shutil
g, threads = sys.argv[1], int(sys.argv[2])
tmp = f"data/duck-{g}"; shutil.rmtree(tmp, ignore_errors=True); os.makedirs(tmp)
con = duckdb.connect(f"{tmp}/w.duckdb"); con.execute(f"SET threads={threads}; SET memory_limit='24GB'; SET temp_directory='{tmp}/tmp'; SET preserve_insertion_order=false")
t0 = time.time(); steps = {}
def step(name, sql):
    t = time.time(); con.execute(sql); steps[name] = round(time.time() - t, 2)
step("read_csv", f"CREATE TABLE e AS SELECT source::BIGINT s, target::BIGINT t FROM read_csv('data/{g}.csv', header=true)")
step("ids", "CREATE TABLE ids AS SELECT id, (row_number() OVER (ORDER BY id) - 1)::UINTEGER AS k FROM (SELECT s id FROM e UNION SELECT t FROM e)")
step("relabel_sym_dedup_sort", f"""COPY (SELECT DISTINCT a, b FROM (
    SELECT i1.k a, i2.k b FROM e JOIN ids i1 ON e.s = i1.id JOIN ids i2 ON e.t = i2.id WHERE e.s <> e.t
    UNION ALL SELECT i2.k, i1.k FROM e JOIN ids i1 ON e.s = i1.id JOIN ids i2 ON e.t = i2.id WHERE e.s <> e.t)
    ORDER BY a, b) TO '{tmp}/adj.parquet' (FORMAT PARQUET, COMPRESSION ZSTD)""")
rows = con.execute(f"SELECT count(*) FROM '{tmp}/adj.parquet'").fetchone()[0]
r = dict(tool="duckdb", version=duckdb.__version__, graph=g, threads=threads, task="ingest_csv_to_sorted_adjacency_parquet",
         seconds=round(time.time() - t0, 2), steps=steps, entries=rows, parquet_mb=round(os.path.getsize(f"{tmp}/adj.parquet") / 2**20),
         peak_rss_mb=round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024))
print(json.dumps(r)); open("results/baselines.jsonl", "a").write(json.dumps(r) + "\n"); con.close(); shutil.rmtree(tmp)
