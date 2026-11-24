#!/usr/bin/env python
"""DuckDB "equivalent ingest": SNAP .txt.gz edge list -> dense-id, symmetrized, deduplicated,
self-loop-free, (src,dst)-sorted edge table written as Parquet (src UINTEGER, dst UINTEGER).

usage: duckdb_ingest.py <lj|orkut> [threads=32] [out_json]
       duckdb_ingest.py <lj|orkut> --verify      (compare kept parquet against the glab-csr-v0 files)
env:   GLAB_DUCK_MEM (default 6GB), GLAB_DUCK_KEEP=1 keeps py/tmp output (default: deleted)
"""
import json, os, resource, shutil, sys, time
import duckdb

name = sys.argv[1]
assert name in ("lj", "orkut")
HERE = os.path.dirname(os.path.abspath(__file__))
RAW = f"/home/tux/graphlab/data/raw/com-{name}.ungraph.txt.gz"
CSR = f"/home/tux/graphlab/data/csr/{name}"
WORK = os.path.join(HERE, "tmp", f"duck_{name}")
OUT = os.path.join(WORK, "edges.parquet")
EXPECT = {"lj": (3997962, 69362378), "orkut": (3072441, 234370166)}[name]


def peak_gb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1048576


if len(sys.argv) > 2 and sys.argv[2] == "--verify":
    import numpy as np
    con = duckdb.connect()
    con.execute("SET memory_limit='1GB'")
    con.execute("SET threads=4")
    offsets = np.fromfile(f"{CSR}/offsets.u64", dtype="<u8")
    targets = np.memmap(f"{CSR}/targets.u32", dtype="<u4", mode="r")
    n = len(offsets) - 1
    ok_dst = ok_src = True
    deg = np.zeros(n, dtype=np.int64)
    prev_last = -1
    step = 20_000_000
    total = con.execute(f"SELECT count(*) FROM read_parquet('{OUT}')").fetchone()[0]
    assert total == len(targets), (total, len(targets))
    for lo in range(0, total, step):
        a = con.execute(
            f"SELECT src, dst FROM read_parquet('{OUT}', file_row_number=true) "
            f"WHERE file_row_number >= {lo} AND file_row_number < {lo + step} ORDER BY file_row_number"
        ).fetchnumpy()
        s, d = a["src"], a["dst"]
        ok_dst &= bool(np.array_equal(d, targets[lo:lo + len(d)]))
        deg += np.bincount(s, minlength=n)
        ok_src &= bool(np.all(np.diff(s.astype(np.int64)) >= 0)) and int(s[0]) >= prev_last
        prev_last = int(s[-1])
    ok_deg = bool(np.array_equal(deg, np.diff(offsets).astype(np.int64)))
    print(json.dumps({"verify": name, "rows": total, "dst_equals_csr_targets": ok_dst,
                      "src_nondecreasing": ok_src, "degrees_equal_csr_offsets": ok_deg,
                      "peak_rss_gb": peak_gb()}))
    sys.exit(0 if (ok_dst and ok_src and ok_deg) else 1)

threads = int(sys.argv[2]) if len(sys.argv) > 2 else 32
out_json = sys.argv[3] if len(sys.argv) > 3 else None
mem = os.environ.get("GLAB_DUCK_MEM", "6GB")
keep = os.environ.get("GLAB_DUCK_KEEP") == "1"

shutil.rmtree(WORK, ignore_errors=True)
os.makedirs(os.path.join(WORK, "spill"))
load0 = os.getloadavg()

T0 = time.perf_counter()
con = duckdb.connect(os.path.join(WORK, "work.duckdb"))
con.execute(f"PRAGMA threads={threads}")
con.execute(f"SET memory_limit='{mem}'")
con.execute(f"SET temp_directory='{os.path.join(WORK, 'spill')}'")
stages = {}


def stage(label, sql):
    t = time.perf_counter()
    r = con.execute(sql).fetchall()
    stages[label] = time.perf_counter() - t
    print(f"[{label}] {stages[label]:.2f}s", flush=True)
    return r


# 1. parse the gzip text
stage("read_csv", f"""
CREATE TABLE raw AS
SELECT u, v FROM read_csv('{RAW}', delim='\t', comment='#', header=false,
                          columns={{'u': 'BIGINT', 'v': 'BIGINT'}}, auto_detect=false)""")
# 2. dense ids = rank of sorted distinct original ids
stage("ids", """
CREATE TABLE ids AS
SELECT id, (row_number() OVER (ORDER BY id) - 1)::UINTEGER AS dense
FROM (SELECT u AS id FROM raw UNION SELECT v AS id FROM raw)""")
n = con.execute("SELECT count(*) FROM ids").fetchone()[0]
# 3. relabel, drop self loops, symmetrize, dedup, sort, write
r = stage("relabel_sym_dedup_sort_write", f"""
COPY (
  WITH e AS (
    SELECT a.dense AS s, b.dense AS d
    FROM raw JOIN ids a ON raw.u = a.id JOIN ids b ON raw.v = b.id
    WHERE raw.u <> raw.v)
  SELECT DISTINCT src, dst
  FROM (SELECT s AS src, d AS dst FROM e UNION ALL SELECT d AS src, s AS dst FROM e)
  ORDER BY src, dst
) TO '{OUT}' (FORMAT parquet)""")
entries = r[0][0]
seconds = time.perf_counter() - T0
con.close()

res = {
    "dataset": name,
    "seconds": seconds,
    "stage_seconds": stages,
    "n": n,
    "entries": entries,
    "expected_n": EXPECT[0],
    "expected_entries": EXPECT[1],
    "counts_match": (n, entries) == EXPECT,
    "threads": threads,
    "cpu_affinity_count": len(os.sched_getaffinity(0)),
    "memory_limit": mem,
    "duckdb_version": duckdb.__version__,
    "output_kind": "parquet file (src UINTEGER, dst UINTEGER; default snappy; rows ordered by src,dst) "
                   "written by COPY; intermediates (raw, ids) are tables in an on-disk DuckDB file",
    "output_bytes": os.path.getsize(OUT),
    "workdb_bytes": os.path.getsize(os.path.join(WORK, "work.duckdb")),
    "peak_rss_gb": peak_gb(),
    "loadavg_start": load0,
    "loadavg_end": os.getloadavg(),
}
print(json.dumps(res, indent=1))
if out_json:
    os.makedirs(os.path.dirname(out_json), exist_ok=True)
    with open(out_json, "w") as f:
        json.dump(res, f, indent=1)
    print("wrote", out_json)
if not keep:
    shutil.rmtree(WORK, ignore_errors=True)
assert res["counts_match"], (n, entries, EXPECT)
