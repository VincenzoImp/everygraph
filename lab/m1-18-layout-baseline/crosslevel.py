#!/usr/bin/env python3
"""crosslevel.py CSR HIER OUT.json NAME=POS.f32 ... — where the long edges come from.

For each hierarchy level: the share of edges whose ends lie in different communities, the
size share of the largest community, and per layout the median length of edges inside and
across communities, each divided by the layout's median random-pair distance (10^6 pairs,
seed 1).
"""
import json, sys
import numpy as np

g, h, out = sys.argv[1], sys.argv[2], sys.argv[3]
n = json.load(open(f"{g}/meta.json"))["n"]
off = np.fromfile(f"{g}/offsets.u64", dtype=np.uint64).astype(np.int64)
t = np.fromfile(f"{g}/targets.u32", dtype=np.uint32)
s = np.repeat(np.arange(n, dtype=np.uint32), np.diff(off))
k = t > s
u, v = s[k].astype(np.int64), t[k].astype(np.int64)
del s, t, k
rng = np.random.default_rng(1)
a, b = rng.integers(0, n, 10**6), rng.integers(0, n, 10**6)
lay = {}
for arg in sys.argv[4:]:
    nm, path = arg.split("=", 1)
    p = np.fromfile(path, dtype=np.float32).reshape(-1, 2)
    lay[nm] = (np.hypot(*(p[u] - p[v]).T), np.median(np.hypot(*(p[a] - p[b]).T)))
levels = []
c = np.fromfile(f"{h}/level_0.u32", dtype=np.uint32).astype(np.int64)
for lv in range(len(json.load(open(f"{h}/hier.json"))["levels"])):
    if lv:
        c = np.fromfile(f"{h}/level_{lv}.u32", dtype=np.uint32).astype(np.int64)[c]
    x = c[u] != c[v]
    row = {"level": lv, "communities": int(c.max()) + 1, "cross_edge_share": float(x.mean()),
           "largest_community_share": float(np.bincount(c).max() / n)}
    for nm, (el, r) in lay.items():
        row[nm] = {"inside_median_ratio": float(np.median(el[~x]) / r), "across_median_ratio": float(np.median(el[x]) / r)}
    levels.append(row)
    print(json.dumps(row), flush=True)
json.dump({"graph": g, "hier": h, "levels": levels}, open(out, "w"), indent=1)
