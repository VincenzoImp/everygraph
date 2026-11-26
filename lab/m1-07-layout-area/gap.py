#!/usr/bin/env python3
"""gap.py CSR HIER LEVEL OUT.json NAME=POS.f32 ... — where the length of edges between
top regions comes from.

For every edge whose ends lie in different level-LEVEL regions A and B:
  L  its length;
  C  the distance between the centroids of A and B (member positions);
  G  max(0, C - R_A - R_B), R = sqrt(2) * rms distance of the members to their centroid
     (the radius of a uniform disc with the same spread). G is what is left if every
     member with an outside link sat on its region's rim facing the other region:
     the floor that within-region placement can reach while the regions stay where they are.
All three are divided by the layout's median random-pair distance (10^6 pairs, seed 1,
as crosslevel.py). Also: the share of crossing edges that go to one of the region's
3 nearest regions (by centroid), the median over edges of B's rank among A's neighbours
by distance, and "facing": the mean offset of an edge's end from its region's centroid
towards the other region, in units of R (0 when members sit regardless of their outside
links; 0.42 if they fill the facing half of a uniform disc, 1 if all sit on the facing rim).
"""
import json, sys
import numpy as np


def elen(p, u, v):
    """edge lengths in chunks, to bound memory on orkut"""
    out = np.empty(len(u))
    for i in range(0, len(u), 1 << 24):
        out[i:i + (1 << 24)] = np.hypot(*(p[u[i:i + (1 << 24)]] - p[v[i:i + (1 << 24)]]).T)
    return out


g, h, level, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
n = json.load(open(f"{g}/meta.json"))["n"]
off = np.fromfile(f"{g}/offsets.u64", dtype=np.uint64).astype(np.int64)
t = np.fromfile(f"{g}/targets.u32", dtype=np.uint32)
s = np.repeat(np.arange(n, dtype=np.uint32), np.diff(off))
k = t > s
u, v = s[k], t[k]
del s, t, k
c = np.fromfile(f"{h}/level_0.u32", dtype=np.uint32).astype(np.int64)
for lv in range(1, level + 1):
    c = np.fromfile(f"{h}/level_{lv}.u32", dtype=np.uint32).astype(np.int64)[c]
nc = int(c.max()) + 1
x = c[u] != c[v]
u, v = u[x], v[x]
ca, cb = c[u], c[v]
cnt = np.bincount(c, minlength=nc).astype(np.float64)
rng = np.random.default_rng(1)
ra, rb = rng.integers(0, n, 10**6), rng.integers(0, n, 10**6)
res = {"graph": g, "level": level, "regions": nc, "crossing_edges": int(x.sum()), "share_of_edges": float(x.mean()), "layouts": {}}
for arg in sys.argv[5:]:
    nm, path = arg.split("=", 1)
    p = np.fromfile(path, dtype=np.float32).reshape(-1, 2).astype(np.float64)
    r50 = np.median(np.hypot(*(p[ra] - p[rb]).T))
    cen = np.stack([np.bincount(c, p[:, 0], nc), np.bincount(c, p[:, 1], nc)], 1) / cnt[:, None]
    ms = np.bincount(c, ((p - cen[c]) ** 2).sum(1), nc) / cnt
    R = np.sqrt(2 * ms)
    L = elen(p, u, v)
    C = np.hypot(*(cen[ca] - cen[cb]).T)
    G = np.maximum(0, C - R[ca] - R[cb])
    # rank of B among the regions by centroid distance from A (1 = nearest)
    dd = np.hypot(*(cen[:, None, :] - cen[None, :, :]).transpose(2, 0, 1))
    np.fill_diagonal(dd, np.inf)
    rank = np.argsort(np.argsort(dd, axis=1, kind="stable"), axis=1, kind="stable") + 1
    rk = rank[ca, cb]
    row = {"L_median": float(np.median(L) / r50), "C_median": float(np.median(C) / r50), "G_median": float(np.median(G) / r50),
           "L_mean": float(L.mean() / r50), "C_mean": float(C.mean() / r50), "G_mean": float(G.mean() / r50),
           "share_to_3_nearest_regions": float((rk <= 3).mean()), "median_rank_of_other_region": float(np.median(rk)),
           "region_R_median_per_node": float(np.median(R[c]) / r50)}
    # facing: offset of each end from its region's centroid along the direction to the other
    # region, in units of R; 0 when members sit regardless of their outside links
    dirn = cen[cb] - cen[ca]
    dirn /= np.maximum(np.hypot(*dirn.T), 1e-300)[:, None]
    fu = ((p[u] - cen[ca]) * dirn).sum(1) / np.maximum(R[ca], 1e-300)
    fv = -((p[v] - cen[cb]) * dirn).sum(1) / np.maximum(R[cb], 1e-300)
    row["facing_mean"] = float(np.concatenate([fu, fv]).mean())
    del fu, fv, dirn
    res["layouts"][nm] = row
    print(nm, json.dumps({a: round(b, 4) for a, b in row.items()}), flush=True)
json.dump(res, open(out, "w"), indent=1)
