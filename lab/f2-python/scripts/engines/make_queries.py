import json, numpy as np, sys
from common import RES
name = sys.argv[1] if len(sys.argv) > 1 else "lj"
meta = json.load(open(f"/home/tux/graphlab/data/csr/{name}/meta.json"))
n = meta["n"]; rng = np.random.default_rng(42)
q = {"graph": name, "n": n, "seed": 42,
     "nodes": rng.integers(0, n, 200).tolist(),
     "nodes3": rng.integers(0, n, 50).tolist(),
     "pairs": rng.integers(0, n, (50, 2)).tolist()}
json.dump(q, open(f"{RES}/queries_{name}.json", "w"))
print("ok", n)
