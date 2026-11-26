#!/usr/bin/env python3
"""quick.py CSR HIER NAME=POS.f32 ... — fast screening of layouts on one graph.

Prints per layout: median and mean edge ratio (as metrics.py: all edges, 10^7 random pairs,
seed 1), the median length of edges between top-level regions / random-pair median (as
crosslevel.py: 10^6 pairs, seed 1), and Spearman's rho of layout vs hop distance on
100 BFS sources x 1,000 targets (as metrics.py, seed 4). The numbers that are recorded come
from metrics.py and crosslevel.py; this script only screens variants.
"""
import json, sys
import numpy as np


def elen(p, u, v):
    """edge lengths in chunks, to bound memory on orkut"""
    out = np.empty(len(u))
    for i in range(0, len(u), 1 << 24):
        out[i:i + (1 << 24)] = np.hypot(*(p[u[i:i + (1 << 24)]] - p[v[i:i + (1 << 24)]]).T)
    return out

from scipy.sparse import csr_matrix
from scipy.sparse.csgraph import shortest_path
from scipy.stats import spearmanr

g, h = sys.argv[1], sys.argv[2]
n = json.load(open(f"{g}/meta.json"))["n"]
off = np.fromfile(f"{g}/offsets.u64", dtype=np.uint64).astype(np.int64)
t = np.fromfile(f"{g}/targets.u32", dtype=np.uint32)
s = np.repeat(np.arange(n, dtype=np.uint32), np.diff(off))
k = t > s
u, v = s[k], t[k]
del s, k
c = np.fromfile(f"{h}/level_0.u32", dtype=np.uint32).astype(np.int64)
for lv in range(1, len(json.load(open(f"{h}/hier.json"))["levels"])):
    c = np.fromfile(f"{h}/level_{lv}.u32", dtype=np.uint32).astype(np.int64)[c]
x = c[u] != c[v]
rng = np.random.default_rng(1)
ra, rb = rng.integers(0, n, 10_000_000), rng.integers(0, n, 10_000_000)
gr = csr_matrix((np.ones(len(t), dtype=np.float32), t, off), shape=(n, n))
rng = np.random.default_rng(4)
src = rng.choice(n, 100, replace=False)
D, T = [], []
for i in range(0, 100, 10):
    b = src[i:i + 10]
    d = shortest_path(gr, unweighted=True, directed=False, indices=b)
    for row, s0 in zip(d, b):
        tt = rng.integers(0, n, 1000)
        dd = row[tt]
        ok = np.isfinite(dd) & (dd > 0)
        D.append(dd[ok])
        T.append(np.stack([np.full(ok.sum(), s0), tt[ok]], 1))
D, T = np.concatenate(D), np.concatenate(T)
for arg in sys.argv[3:]:
    nm, path = arg.split("=", 1)
    p = np.fromfile(path, dtype=np.float32).reshape(-1, 2).astype(np.float64)
    el = elen(p, u, v)
    rl = np.hypot(*(p[ra] - p[rb]).T)
    r50 = np.median(rl)
    r50m = np.median(np.hypot(*(p[ra[:1_000_000]] - p[ra[1_000_000:2_000_000]]).T))
    E = np.hypot(*(p[T[:, 0]] - p[T[:, 1]]).T)
    print(json.dumps({"layout": nm, "median_ratio": round(np.median(el) / r50, 4), "mean_ratio": round(el.mean() / rl.mean(), 4),
                      "top_cross_median": round(np.median(el[x]) / r50m, 4), "spearman": round(float(spearmanr(D, E).statistic), 4)}), flush=True)
