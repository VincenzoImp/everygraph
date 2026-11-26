#!/usr/bin/env python3
"""facing.py CSR HIER LEVEL POS.f32 OUT.json — can members face their outside links?

Take the edges between level-LEVEL regions, with each region at the centroid of its
members in the layout POS. Each end u of such an edge (u in A, v in B) gets the unit
vector d from A's centroid towards B's. For a group X of A's members (a node, or a
level-k community inside A), |mean of d over X's crossing edges| is the most that X can
face its outside links: placed on A's rim in the best direction, the mean offset of X's
edge ends towards the other region is |mean d| * R_A. The script reports, per level k
below LEVEL, the crossing-edge-weighted mean of |mean d|, for real groups and for groups
of the same sizes with the directions shuffled across all crossing edges (seed 1), which
is the value that pure chance gives a group of that size.

"centred" removes from d the mean direction of all of A's crossing edges first. The
members' mean offset from A's centroid is zero, so only the part of a group's direction
that differs from its region's mean can be faced: this is the tighter bound.
"""
import json, sys
import numpy as np

g, h, level, posf, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5]
n = json.load(open(f"{g}/meta.json"))["n"]
off = np.fromfile(f"{g}/offsets.u64", dtype=np.uint64).astype(np.int64)
t = np.fromfile(f"{g}/targets.u32", dtype=np.uint32)
s = np.repeat(np.arange(n, dtype=np.uint32), np.diff(off))
lab = [np.fromfile(f"{h}/level_0.u32", dtype=np.uint32).astype(np.int64)]
for lv in range(1, level + 1):
    lab.append(np.fromfile(f"{h}/level_{lv}.u32", dtype=np.uint32).astype(np.int64)[lab[-1]])
c = lab[level]
# every directed edge (s, t) that crosses: end s gets the direction towards t's region
x = c[s] != c[t]
s, t = s[x], t[x]
p = np.fromfile(posf, dtype=np.float32).reshape(-1, 2).astype(np.float64)
nc = int(c.max()) + 1
cnt = np.bincount(c, minlength=nc)
cen = np.stack([np.bincount(c, p[:, 0], nc), np.bincount(c, p[:, 1], nc)], 1) / np.maximum(cnt, 1)[:, None]
d = cen[c[t]] - cen[c[s]]
d /= np.maximum(np.hypot(*d.T), 1e-300)[:, None]
del t
shuf = d[np.random.default_rng(1).permutation(len(d))]
cs = c[s]
ne = np.maximum(np.bincount(cs, minlength=nc), 1)
dc = d - np.stack([np.bincount(cs, d[:, 0], nc), np.bincount(cs, d[:, 1], nc)], 1)[cs] / ne[cs][:, None]
del cs
res = {"graph": g, "level": level, "pos": posf, "crossing_edge_ends": int(len(s)), "groups": []}


def facing(key, dd):
    k, inv = np.unique(key, return_inverse=True)
    sx = np.bincount(inv, dd[:, 0], len(k))
    sy = np.bincount(inv, dd[:, 1], len(k))
    return float(np.hypot(sx, sy).sum() / len(dd)), len(k)


for name, key in [("node", s.astype(np.int64))] + [(f"L{k}", lab[k][s]) for k in range(level)]:
    real, groups = facing(key, d)
    rand, _ = facing(key, shuf)
    cent, _ = facing(key, dc)
    res["groups"].append({"group": name, "groups_with_crossing_edges": groups, "facing_bound": real, "chance": rand, "centred_bound": cent})
    print(json.dumps(res["groups"][-1]), flush=True)
json.dump(res, open(out, "w"), indent=1)
