#!/usr/bin/env python3
"""depth_facing.py CSR HIER LEVEL LAYOUT_DIR... — at which tree depth does facing appear?

For each gmap layout output (pos.f32, tnodes.bin, leafparent.u32): the ends of the edges
between level-LEVEL regions (as facing.py), and each end's chain of tree ancestors. The
offset of an end from its top region's centre is the sum of the steps centre(depth k) -
centre(depth k-1) and finally pos - centre(leaf parent). Printed per depth: the mean of
each step projected on the direction to the other region (member centroids, as gap.py),
in units of the region's R (sqrt(2) * rms member distance). Their sum is the facing of
gap.py, up to the difference between the tree's disc centre and the member centroid.
"""
import json, sys
import numpy as np

TN = np.dtype([("parent", "<u4"), ("start", "<u4"), ("end", "<u4"), ("comm", "<u4"), ("level", "i1"), ("depth", "u1"),
               ("flags", "u1"), ("pad", "u1"), ("x", "<f4"), ("y", "<f4"), ("r", "<f4"), ("node_r", "<f4"),
               ("nkids", "<u4"), ("internal", "<u8"), ("volume", "<u8")])
g, h, level = sys.argv[1], sys.argv[2], int(sys.argv[3])
n = json.load(open(f"{g}/meta.json"))["n"]
off = np.fromfile(f"{g}/offsets.u64", dtype=np.uint64).astype(np.int64)
t = np.fromfile(f"{g}/targets.u32", dtype=np.uint32)
s = np.repeat(np.arange(n, dtype=np.uint32), np.diff(off))
c = np.fromfile(f"{h}/level_0.u32", dtype=np.uint32).astype(np.int64)
for lv in range(1, level + 1):
    c = np.fromfile(f"{h}/level_{lv}.u32", dtype=np.uint32).astype(np.int64)[c]
x = c[s] != c[t]
s, t = s[x], t[x]
rng = np.random.default_rng(1)
pick = np.sort(rng.choice(len(s), min(len(s), 2_000_000), replace=False))
s, t = s[pick].astype(np.int64), t[pick].astype(np.int64)
nc = int(c.max()) + 1
cnt = np.maximum(np.bincount(c, minlength=nc), 1)
for lay in sys.argv[4:]:
    p = np.fromfile(f"{lay}/pos.f32", dtype=np.float32).reshape(-1, 2).astype(np.float64)
    tn = np.fromfile(f"{lay}/tnodes.bin", dtype=TN)
    lp = np.fromfile(f"{lay}/leafparent.u32", dtype=np.uint32).astype(np.int64)
    cen = np.stack([np.bincount(c, p[:, 0], nc), np.bincount(c, p[:, 1], nc)], 1) / cnt[:, None]
    R = np.sqrt(2 * np.bincount(c, ((p - cen[c]) ** 2).sum(1), nc) / cnt)
    d = cen[c[t]] - cen[c[s]]
    d /= np.maximum(np.hypot(*d.T), 1e-300)[:, None]
    xy = np.stack([tn["x"], tn["y"]], 1).astype(np.float64)
    par = tn["parent"].astype(np.int64)
    dep = tn["depth"].astype(np.int64)
    a = lp[s]
    steps = {"node": (((p[s] - xy[a]) * d).sum(1) / R[c[s]]).mean()}
    while True:
        live = dep[a] > 0
        if not live.any():
            break
        pa = np.where(live, par[a], a)
        st = np.where(live, ((xy[a] - xy[pa]) * d).sum(1), 0.0) / R[c[s]]
        for k in np.unique(dep[a][live]):
            m = live & (dep[a] == k)
            steps[f"depth {k}"] = steps.get(f"depth {k}", 0.0) + st[m].sum() / len(s)
        a = pa
    print(lay.rsplit("/", 1)[-1], json.dumps({k: round(float(v), 4) for k, v in sorted(steps.items())}), flush=True)
