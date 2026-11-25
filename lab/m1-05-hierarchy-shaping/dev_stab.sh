#!/usr/bin/env bash
# Stability (exploratory): drop 1% of the undirected edges (seeded hash), rebuild the plain
# partition and every method on the perturbed graph, and compare the top levels with the
# unperturbed trees (ARI and NMI per depth, over all nodes). Run after dev_all.sh.
#   ./dev_stab.sh GRAPH STORE [METHODS...]      -> results/dev/GRAPH.stab.json
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
g=$1; store=$2; shift 2
methods=${*:-capin sat capafter rec nest}
PY=${PY:-$S/venv/bin/python}
LIM=${LIM:-8G}
M=../m1-02-ooc-leiden/target/release/m102
P=$S/stores/$g-drop1
mkdir -p "$S/stores"
[ -d "$P" ] || ./lim.sh $LIM $M cutgraph --store "$store" --out "$P" --drop 0.01 --seed 7 --threads 16 > "$S/stores/$g-drop1.json"
S=$S ./dev_all.sh "$g-drop1" "$P" $methods > /dev/null
out=results/dev/$g.stab${TAG:-}.json
echo "{" > "$out"
$PY -I - "$S/$g/plain.memb.u32" "$S/$g-drop1/plain.memb.u32" >> "$out" <<'EOF'
import json, sys
import numpy as np
sys.path.insert(0, ".")
from shape import ari_nmi
a, b = (np.fromfile(p, np.uint32) for p in sys.argv[1:])
ari, nmi = ari_nmi(a, b)
print(json.dumps("plain") + ": " + json.dumps({"ari": ari, "nmi": nmi, "communities_a": int(len(np.unique(a))), "communities_b": int(len(np.unique(b)))}) + ",")
EOF
first=1
for m in $methods; do
  [ $first = 1 ] || echo "," >> "$out"
  first=0
  printf "\"%s\": " "$m${TAG:-}" >> "$out"
  ./lim.sh $LIM $PY -I shape.py compare --tree "$S/$g/tree_$m${TAG:-}.npz" --tree "$S/$g-drop1/tree_$m${TAG:-}.npz" --max-depth 4 >> "$out"
done
echo "}" >> "$out"
python3 -I - "$out" <<'EOF'
import json, sys
d = json.load(open(sys.argv[1]))
print("plain T: ARI %.3f NMI %.3f" % (d["plain"]["ari"], d["plain"]["nmi"]))
for m, v in d.items():
    if m == "plain":
        continue
    print(m, "  ".join("d%s ARI %.3f NMI %.3f (%d/%d)" % (k, x["ari"], x["nmi"], x["regions_a"], x["regions_b"]) for k, x in v["by_depth"].items()))
EOF
