#!/usr/bin/env bash
# The isolated-vertex stopping defect (exploratory-deterministic): the plain deterministic
# Leiden on lj and orkut with 1% of their edges dropped (seed 11), with the m102 of b48f822
# (before the fix: isolated vertices counted in the pass-stopping test) and with this branch's.
#   OLD=<m102 built from b48f822's leiden.rs> ./dev_stopfix.sh > results/stopfix.jsonl
# (build: copy lab/m1-02-ooc-leiden to a scratch dir, `git show b48f822:lab/m1-02-ooc-leiden/src/leiden.rs`
# over src/leiden.rs, make the everygraph-core path absolute, cargo build --release)
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
NEW=../m1-02-ooc-leiden/target/release/m102
mkdir -p "$S/stopfix"
for g in lj orkut; do
  P=$S/stopfix/$g-d11
  [ -d "$P" ] || ./lim.sh 8G $NEW cutgraph --store /home/tux/graphlab/data/store-m113/$g --out "$P" --drop 0.01 --seed 11 --out-deg "$P.deg.u32" --threads 16 > /dev/null
  iso=$(python3 -I -c "import sys; b=open(sys.argv[1],'rb').read(); import array; a=array.array('I'); a.frombytes(b); print(sum(1 for x in a if x == 0))" "$P.deg.u32")
  for which in old new; do
    bin=$NEW; [ $which = old ] && bin=$OLD
    ./lim.sh 8G $bin leiden run --config comp --store "$P" --mem mmap --threads 16 --det --out-memb "$S/stopfix/memb.u32" 2>/dev/null | tail -1 > "$S/stopfix/run.json"
    python3 -I - "$S/stopfix/run.json" "$S/stopfix/memb.u32" $g $which $iso <<'EOF'
import array, collections, json, sys
d = json.load(open(sys.argv[1]))
a = array.array("I"); a.frombytes(open(sys.argv[2], "rb").read())
c = sorted(collections.Counter(a).values(), reverse=True)
tot, acc, cover = sum(c), 0, 0
for x in c:
    acc += x; cover += 1
    if acc >= 0.9 * tot:
        break
iso = int(sys.argv[5])
print(json.dumps({"graph": sys.argv[3], "perturbation": "1% of edges dropped, seed 11", "binary": sys.argv[4], "isolated_nodes": iso,
                  "communities": d["communities"], "communities_not_isolated": d["communities"] - iso, "passes": d["n_passes"],
                  "level_sizes": d["level_sizes"], "modularity": d["modularity"], "cover90": cover, "hash_levels": d["hash_levels"]}))
EOF
  done
done
