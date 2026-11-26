#!/usr/bin/env python3
"""fa2_cugraph.py CSR_DIR OUT.f32 [MAX_ITER] — ForceAtlas2 (Jacomy et al.) as implemented in
RAPIDS cuGraph, on the GPU, with cuGraph's defaults (Barnes-Hut theta 0.5, scaling 2,
gravity 1, outbound attraction distribution on). A reference run only: the product never
needs a GPU. Writes n x 2 f32 in dense node order and prints a JSON line.
"""
import json, sys, time
import numpy as np
import cudf, cugraph

csr, out = sys.argv[1], sys.argv[2]
it = int(sys.argv[3]) if len(sys.argv) > 3 else 500
n = json.load(open(f"{csr}/meta.json"))["n"]
off = np.fromfile(f"{csr}/offsets.u64", dtype=np.uint64).astype(np.int64)
tgt = np.fromfile(f"{csr}/targets.u32", dtype=np.uint32).astype(np.int32)
src = np.repeat(np.arange(n, dtype=np.int32), np.diff(off))
keep = tgt > src
t0 = time.time()
df = cudf.DataFrame({"src": src[keep], "dst": tgt[keep]})
G = cugraph.Graph(directed=False)
G.from_cudf_edgelist(df, source="src", destination="dst", renumber=False)
t1 = time.time()
pos = cugraph.force_atlas2(G, max_iter=it).to_pandas()
t2 = time.time()
xy = np.full((n, 2), np.nan, dtype=np.float32)
xy[pos["vertex"].to_numpy()] = pos[["x", "y"]].to_numpy()
xy.tofile(out)
print(json.dumps({"tool": "cuGraph force_atlas2", "cugraph": cugraph.__version__, "max_iter": it,
                  "build_s": round(t1 - t0, 2), "layout_s": round(t2 - t1, 2)}))
