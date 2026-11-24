# Bootstrap hierarchy with igraph multilevel (Louvain). Writes B2's hier format.
import sys, json, time, numpy as np, igraph as ig
g, out = sys.argv[1], sys.argv[2]
d = f"/home/tux/graphlab/data/csr/{g}"
meta = json.load(open(d + "/meta.json")); n = meta["n"]
off = np.fromfile(d + "/offsets.u64", dtype=np.uint64).astype(np.int64)
tg = np.fromfile(d + "/targets.u32", dtype=np.uint32)
src = np.repeat(np.arange(n, dtype=np.uint32), np.diff(off))
mask = src < tg
edges = np.stack([src[mask], tg[mask]], axis=1)
del src, tg
t = time.time()
G = ig.Graph(n=n, edges=edges, directed=False)
print("graph built", time.time() - t, flush=True)
import random; random.seed(1)
t = time.time()
levels = G.community_multilevel(return_levels=True)
el = time.time() - t
print("louvain", el, [len(set(l.membership)) for l in levels], flush=True)
import os; os.makedirs(out, exist_ok=True)
prev = None; cs = []
for k, l in enumerate(levels):
    mem = np.asarray(l.membership, dtype=np.uint32)
    if k == 0:
        mem.tofile(f"{out}/level_0.u32"); cs.append(int(mem.max()) + 1)
    else:
        par = np.zeros(cs[-1], dtype=np.uint32); par[prev] = mem
        par.tofile(f"{out}/level_{k}.u32"); cs.append(int(mem.max()) + 1)
    prev = mem
json.dump({"graph": g, "n": n, "levels": cs, "method": "igraph community_multilevel", "seconds": el,
           "modularity": [float(l.modularity) for l in levels]}, open(f"{out}/hier.json", "w"))
