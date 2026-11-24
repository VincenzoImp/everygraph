#!/usr/bin/env python
"""NetworKit reference point: PageRank (20 iters) + connected components on a glab-csr-v0 graph.

usage: nk_ref.py <threads> [name=orkut] [out_json]
"""
import json, os, resource, sys, time
import numpy as np

threads = int(sys.argv[1])
name = sys.argv[2] if len(sys.argv) > 2 else "orkut"
out = sys.argv[3] if len(sys.argv) > 3 else None
CSR = f"/home/tux/graphlab/data/csr/{name}"
REPS = 3

import networkit as nk
nk.setNumberOfThreads(threads)


def rss_gb():
    with open("/proc/self/status") as f:
        for line in f:
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1048576
    return float("nan")


def peak_gb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1048576


meta = json.load(open(f"{CSR}/meta.json"))
n = meta["n"]

# ---- load CSR, keep only the u<v half, hand COO to NetworKit --------------
t0 = time.perf_counter()
offsets = np.fromfile(f"{CSR}/offsets.u64", dtype="<u8")
targets = np.memmap(f"{CSR}/targets.u32", dtype="<u4", mode="r")
assert len(offsets) == n + 1 and offsets[-1] == len(targets) == meta["entries"]
deg = np.diff(offsets).astype(np.int64)
src = np.repeat(np.arange(n, dtype=np.uint32), deg)
mask = src < targets
row = src[mask]
del src
col = targets[mask]
del mask, targets
assert len(row) == meta["m_undirected"], (len(row), meta["m_undirected"])
t_load = time.perf_counter() - t0

t1 = time.perf_counter()
G = nk.GraphFromCoo((row, col), n=n, weighted=False, directed=False)
t_coo = time.perf_counter() - t1
del row, col
build_seconds = time.perf_counter() - t0
assert G.numberOfNodes() == n and G.numberOfEdges() == meta["m_undirected"], (
    G.numberOfNodes(), G.numberOfEdges())
assert not G.isDirected() and not G.isWeighted()
rss_after_build = rss_gb()
print(f"[build] n={n} m={G.numberOfEdges()} load_half={t_load:.2f}s GraphFromCoo={t_coo:.2f}s "
      f"total={build_seconds:.2f}s rss={rss_after_build:.2f}GB peak={peak_gb():.2f}GB", flush=True)

# ---- PageRank: damping 0.85, exactly 20 iterations -------------------------
pr_times, pr_iters = [], []
pr = None
for _ in range(REPS):
    pr = nk.centrality.PageRank(G, damp=0.85, tol=1e-300)
    pr.maxIterations = 20
    t = time.perf_counter(); pr.run(); pr_times.append(time.perf_counter() - t)
    pr_iters.append(pr.numberOfIterations())
pr_norm = {0: "L1_NORM", 1: "L2_NORM"}.get(int(pr.norm), str(pr.norm)) + " (library default; only used for the tol stop test, which is disabled by tol=1e-300)"
scores = np.asarray(pr.scores(), dtype=np.float64)
pr_sum = float(scores.sum())
top = np.argsort(-scores, kind="stable")[:5]
top5 = [[int(i), float(scores[i])] for i in top]
print(f"[pagerank] threads={threads} times={['%.4f' % x for x in pr_times]} iters={pr_iters} "
      f"norm={pr_norm} sum={pr_sum!r}", flush=True)
print(f"[pagerank] top5 (dense id, score) = {top5}", flush=True)
assert all(i == 20 for i in pr_iters), pr_iters
del scores, pr

# ---- connected components --------------------------------------------------
cc_times = []
for _ in range(REPS):
    cc = nk.components.ConnectedComponents(G)
    t = time.perf_counter(); cc.run(); cc_times.append(time.perf_counter() - t)
n_comp = int(cc.numberOfComponents())
del cc
print(f"[cc] times={['%.4f' % x for x in cc_times]} components={n_comp}", flush=True)

pcc_times, n_comp_par = None, None
if hasattr(nk.components, "ParallelConnectedComponents"):
    pcc_times = []
    for _ in range(REPS):
        pcc = nk.components.ParallelConnectedComponents(G)
        t = time.perf_counter(); pcc.run(); pcc_times.append(time.perf_counter() - t)
    n_comp_par = int(pcc.numberOfComponents())
    del pcc
    print(f"[parallel_cc] times={['%.4f' % x for x in pcc_times]} components={n_comp_par}", flush=True)

res = {
    "dataset": name,
    "n": n,
    "m_undirected": meta["m_undirected"],
    "threads": threads,
    "nk_max_threads": nk.getMaxNumberOfThreads(),
    "cpu_affinity_count": len(os.sched_getaffinity(0)),
    "pagerank_seconds_best": min(pr_times),
    "pagerank_seconds_all": pr_times,
    "pagerank_iterations": pr_iters,
    "pagerank_settings": {"damp": 0.85, "tol": 1e-300, "maxIterations": 20, "norm": pr_norm,
                          "normalized": False, "distributeSinks": "default (NoSinkHandling)"},
    "cc_seconds_best": min(cc_times),
    "cc_seconds_all": cc_times,
    "parallel_cc_seconds_best": min(pcc_times) if pcc_times else None,
    "parallel_cc_seconds_all": pcc_times,
    "n_components": n_comp,
    "n_components_parallel": n_comp_par,
    "build_seconds": build_seconds,
    "build_seconds_numpy_half": t_load,
    "build_seconds_graphfromcoo": t_coo,
    "rss_after_build_gb": rss_after_build,
    "peak_rss_gb": peak_gb(),
    "networkit_version": nk.__version__,
    "numpy_version": np.__version__,
    "top5": top5,
    "pr_sum": pr_sum,
    "loadavg_at_end": os.getloadavg(),
}
print(json.dumps(res, indent=1))
if out:
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w") as f:
        json.dump(res, f, indent=1)
    print("wrote", out)
