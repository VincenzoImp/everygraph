#!/usr/bin/env python3
"""convert.py CSR_DIR OUT_DIR [edges,mtx,dot] — write the undirected edge list of a glab-csr-v0 graph in the
input formats of the reference tools. Dense ids 0..n-1 are kept, so every tool's output can
be read back in CSR order.

  edges.txt   DRGraph and the OGDF driver: "n m" then "u v 1" per edge, u < v, 0-based
  graph.mtx   BatchLayout: MatrixMarket pattern symmetric, lower triangle, 1-based
  graph.dot   sfdp: "graph G { u -- v; ... }"
"""
import json, os, sys, time
import numpy as np

csr, out = sys.argv[1], sys.argv[2]
fmts = sys.argv[3].split(",") if len(sys.argv) > 3 else ["edges", "mtx", "dot"]
os.makedirs(out, exist_ok=True)
meta = json.load(open(f"{csr}/meta.json"))
n = meta["n"]
off = np.fromfile(f"{csr}/offsets.u64", dtype=np.uint64).astype(np.int64)
tgt = np.fromfile(f"{csr}/targets.u32", dtype=np.uint32)
src = np.repeat(np.arange(n, dtype=np.uint32), np.diff(off))
keep = tgt > src
u, v = src[keep], tgt[keep]
del src, tgt, keep
m = len(u)
assert m == meta["m_undirected"], (m, meta["m_undirected"])
print(f"n={n} m={m}", flush=True)

CH = 1 << 20


def write(path, header, fmt, a, b, footer=""):
    t = time.time()
    with open(path, "w") as f:
        f.write(header)
        for i in range(0, m, CH):
            np.savetxt(f, np.stack([a[i:i + CH], b[i:i + CH]], axis=1), fmt=fmt)
        f.write(footer)
    print(path, f"{time.time() - t:.1f}s", flush=True)


if "edges" in fmts:
    write(f"{out}/edges.txt", f"{n} {m}\n", "%d %d 1", u, v)
if "mtx" in fmts:
    write(f"{out}/graph.mtx", f"%%MatrixMarket matrix coordinate pattern symmetric\n{n} {n} {m}\n",
          "%d %d", v.astype(np.int64) + 1, u.astype(np.int64) + 1)
if "dot" in fmts:
    write(f"{out}/graph.dot", "graph G {\n", "%d -- %d;", u, v, "}\n")
