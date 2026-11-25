#!/usr/bin/env bash
# Exploratory runs on one graph (shared machine: quality numbers only, no timing is quoted):
#   ./dev_all.sh GRAPH STORE [METHODS...]
# solve (plain Leiden, deterministic), build every method, metrics, export for gmap.
# Hierarchies go to $S/GRAPH/, metrics to results/dev/GRAPH.<method><TAG>.json.
# A variant: TAG=-up EXTRA="--up on" ./dev_all.sh lj STORE rec
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
g=$1
store=$2
shift 2
methods=${*:-capin capin+agg sat capafter rec nest}
W=$S/$g
PY=${PY:-$S/venv/bin/python}
LIM=${LIM:-8G}
TAG=${TAG:-}
mkdir -p "$W" results/dev
[ -f "$W/plain.memb.u32" ] || ./lim.sh $LIM $PY -I shape.py solve --store "$store" --work "$W"
for m in $methods; do
  v=$m$TAG
  ./lim.sh $LIM $PY -I shape.py build --method $m --tag="$TAG" --store "$store" --work "$W" ${EXTRA:-} > "$W/build_$v.json"
  ./lim.sh $LIM $PY -I shape.py metrics --tree "$W/tree_$v.npz" --store "$store" --graph $g --plain-json "$W/plain.json" --out results/dev/$g.$v.json
  $PY -I shape.py export --tree "$W/tree_$v.npz" --graph $g --out "$W/hier_$v" > /dev/null
done
