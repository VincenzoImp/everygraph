#!/usr/bin/env python3
"""topos.py FORMAT IN N OUT.f32 — convert a tool's layout output to n x 2 f32 in dense order.

FORMAT: drgraph ("n 2" header, then "x y" per node 0..n-1), batchlayout ("x y id" with
1-based id), plain (Graphviz -Tplain: "node NAME x y ..."), random (IN ignored: uniform in
the unit square, seed 1, the floor every metric is read against).
"""
import sys
import numpy as np

fmt, path, n, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
pos = np.full((n, 2), np.nan)
if fmt == "drgraph":
    with open(path) as f:
        assert int(f.readline().split()[0]) == n
        pos[:] = np.loadtxt(f, dtype=np.float64, max_rows=n)
elif fmt == "batchlayout":
    a = np.loadtxt(path, dtype=np.float64)
    pos[a[:, 2].astype(np.int64) - 1] = a[:, :2]
elif fmt == "plain":
    with open(path) as f:
        for line in f:
            if line.startswith("node "):
                p = line.split(maxsplit=5)
                pos[int(p[1])] = float(p[2]), float(p[3])
elif fmt == "random":
    pos[:] = np.random.default_rng(1).random((n, 2))
else:
    sys.exit(f"unknown format {fmt}")
bad = int((~np.isfinite(pos)).any(1).sum())
assert bad == 0, f"{bad} nodes without a finite position"
pos.astype(np.float32).tofile(out)
print(out, n)
