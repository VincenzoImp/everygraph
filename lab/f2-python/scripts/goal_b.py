"""Goal B: binding feasibility — zero-copy, call overhead, GIL release, parallel speed."""
import json, os, sys, time, threading, timeit, subprocess, resource
import numpy as np, pyarrow as pa, polars as pl, pandas as pd
import glabpy

CSR = sys.argv[1] if len(sys.argv) > 1 else "/home/tux/graphlab/data/csr/lj"
THREADS = int(os.environ.get("GLAB_THREADS", "6"))
OUT = sys.argv[2] if len(sys.argv) > 2 else None
R = {"csr": CSR, "threads": THREADS, "python": sys.version.split()[0]}
g = glabpy.Graph.open(CSR)
n = g.n
rng = np.random.default_rng(1)
vs = rng.integers(0, n, 200_000).tolist()

# ---------- 1. zero-copy checks ----------
zc = {}
off = g.offsets_view(); tg = g.targets_view()
zc["offsets_view_writeable"] = bool(off.flags.writeable)
v = 12345
nb = g.neighbors(v)
zc["neighbors_view_is_into_mmap"] = nb.ctypes.data == tg.ctypes.data + 4 * int(off[v])
x, it, rust_ptr = g.pagerank(max_iter=20, tol=0.0, threads=THREADS)
zc["pagerank_numpy_ptr_equals_rust_vec_ptr"] = x.ctypes.data == rust_ptr
col, _ = g.pagerank_arrow(max_iter=20, tol=0.0, threads=THREADS)
rust_buf = col.buffer_addresses()[-1]
pa_arr = pa.array(col)                      # Arrow PyCapsule import
zc["pyarrow_from_capsule_same_buffer"] = pa_arr.buffers()[1].address == rust_buf
np_from_pa = pa_arr.to_numpy(zero_copy_only=True)
zc["numpy_from_pyarrow_same_buffer"] = np_from_pa.ctypes.data == rust_buf
s = pl.Series("pagerank", col)              # polars consumes __arrow_c_array__ directly
zc["polars_series_same_buffer"] = s.to_arrow().buffers()[1].address == rust_buf
t0 = time.perf_counter()
tab = g.pagerank_table(max_iter=20, tol=0.0, threads=THREADS)
t_compute = time.perf_counter() - t0
addrs = tab.buffer_addresses()
t0 = time.perf_counter(); rb = pa.record_batch(tab); t_rb = time.perf_counter() - t0
zc["pyarrow_recordbatch_same_buffers"] = rb.column(1).buffers()[1].address in addrs and rb.column(0).buffers()[1].address in addrs
t0 = time.perf_counter(); df = pl.from_arrow(rb); t_pl = time.perf_counter() - t0
zc["polars_df_same_buffers"] = df["pagerank"].to_arrow().buffers()[1].address in addrs
t0 = time.perf_counter(); pdf = rb.to_pandas(types_mapper=pd.ArrowDtype); t_pd = time.perf_counter() - t0
zc["pandas_arrowdtype_same_buffers"] = pdf["pagerank"].array._pa_array.chunks[0].buffers()[1].address in addrs
pdf2 = pd.DataFrame({"pagerank": x}, copy=False)
zc["pandas_numpy_df_copy_false_same_buffer"] = pdf2["pagerank"].to_numpy().ctypes.data == x.ctypes.data
zc["table_wrap_times_s"] = {"pagerank+ids_compute": t_compute, "pa.record_batch": t_rb, "pl.from_arrow": t_pl, "to_pandas(ArrowDtype)": t_pd}
zc["top5_by_pagerank_polars"] = df.sort("pagerank", descending=True).head(5).to_dicts()
R["zero_copy"] = zc

# ---------- 2. tiny-call overhead ----------
def per_call(stmt, glb, number=200_000):
    best = min(timeit.repeat(stmt, globals=glb, number=1, repeat=5))
    return best / number * 1e9
glb = dict(g=g, vs=vs, off=off, tg=tg, offl=off.tolist() if n < 10_000_000 else None)
pyadj_n = 200_000
ov = {}
ov["python_for_loop_empty_ns"] = per_call("for v in vs: pass", glb)
ov["g.noop(v)_ns"] = per_call("for v in vs: g.noop(v)", glb)
ov["g.degree_of(v)_ns"] = per_call("for v in vs: g.degree_of(v)", glb)
ov["g.neighbors(v)_zero_copy_numpy_view_ns"] = per_call("for v in vs: g.neighbors(v)", glb)
ov["g.neighbors_list(v)_copy_to_pylist_ns"] = per_call("for v in vs: g.neighbors_list(v)", glb)
ov["numpy_slice_tg[off[v]:off[v+1]]_ns"] = per_call("for v in vs: tg[off[v]:off[v+1]]", glb)
va = np.asarray(vs, dtype=np.uint32)
t0 = time.perf_counter(); o2, t2 = g.neighbors_batch(va); dt = time.perf_counter() - t0
ov["neighbors_batch_200k_nodes_ns_per_node"] = dt / len(vs) * 1e9
ov["neighbors_batch_total_entries"] = int(t2.size)
R["call_overhead"] = ov

# ---------- 3. GIL released? ----------
def spin(seconds):
    c = 0; end = time.perf_counter() + seconds
    while time.perf_counter() < end: c += 1
    return c
base = spin(0.5) * 2
done = []
th = threading.Thread(target=lambda: done.append(g.pagerank(max_iter=20, tol=0.0, threads=THREADS)))
t0 = time.perf_counter(); th.start(); c = 0
while th.is_alive(): c += 1
th.join(); dt = time.perf_counter() - t0
R["gil"] = {"pagerank_in_thread_s": dt, "main_thread_loop_iters_during": c,
            "main_thread_iters_per_s_during": c / dt, "main_thread_iters_per_s_idle": base,
            "released": c / dt > 0.3 * base}

# ---------- 4. parallel speed vs native ----------
def best(f, k=3):
    ts = []
    for _ in range(k):
        t0 = time.perf_counter(); f(); ts.append(time.perf_counter() - t0)
    return min(ts)
sp = {}
for th_ in sorted({1, THREADS}):
    sp[f"py_pagerank20_t{th_}_s"] = best(lambda: g.pagerank(max_iter=20, tol=0.0, threads=th_))
    sp[f"py_wcc_t{th_}_s"] = best(lambda: g.wcc(th_))
native = os.path.join(os.path.dirname(__file__), "..", "rust", "target", "release", "glab-native")
for th_ in sorted({1, THREADS}):
    out = subprocess.run([native, CSR, "3"], env={**os.environ, "RAYON_NUM_THREADS": str(th_)},
                         capture_output=True, text=True, check=True).stdout
    j = json.loads(out)
    sp[f"native_pagerank20_t{th_}_s"] = j["pagerank_s"]; sp[f"native_wcc_t{th_}_s"] = j["wcc_s"]
    sp["n_components"] = j["n_components"]
R["parallel"] = sp
R["peak_rss_mb"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
txt = json.dumps(R, indent=1, default=str)
print(txt)
if OUT:
    open(OUT, "w").write(txt)
