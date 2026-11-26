#!/usr/bin/env bash
# Determinism (exploratory): the shaped tree is a fixed function of the graph and T. Builds rec
# on lj and web-google at 4 and 16 threads (and again at 16) and compares the tree arrays.
#   ./dev_determinism.sh > results/determinism.jsonl
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
PY=${PY:-$S/venv/bin/python}
for gs in lj:store-m113 web-google:store-m105; do
  g=${gs%%:*}; st=/home/tux/graphlab/data/${gs##*:}/$g
  for v in t4:4 t16:16 t16b:16; do
    ./lim.sh 8G $PY -I shape.py build --method rec --store "$st" --work "$S/$g" --threads ${v##*:} --tag="-det-${v%%:*}" > /dev/null 2>&1
  done
  $PY -I - "$S/$g" "$g" <<'EOF'
import hashlib, json, sys
import numpy as np
W, g = sys.argv[1:]
h = {}
for v in ["t4", "t16", "t16b"]:
    z = np.load(f"{W}/tree_rec-det-{v}.npz")
    m = hashlib.sha256()
    for k in ["par", "kind", "nn", "ncomm", "leaf"]:
        m.update(np.ascontiguousarray(z[k]).tobytes())
    h[v] = m.hexdigest()[:16]
print(json.dumps({"graph": g, "method": "rec", "tree_sha256_16": h, "identical": len(set(h.values())) == 1, "threads": {"t4": 4, "t16": 16, "t16b": 16}}))
EOF
done
