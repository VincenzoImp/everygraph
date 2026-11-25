#!/usr/bin/env bash
# Exploratory runs on one graph (shared machine: quality numbers only, no timing is quoted):
#   ./dev_all.sh GRAPH STORE [VARIANTS...]
# solve (plain Leiden, deterministic), build every variant, metrics, export for gmap.
# A variant is a method, or a method with "-up" (super level always built: --up on).
# Hierarchies go to $S/GRAPH/, metrics to results/dev/GRAPH.<variant>.json.
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
g=$1
store=$2
shift 2
variants=${*:-capin capin+agg sat capafter rec rec-up nest nest-up}
W=$S/$g
PY=${PY:-$S/venv/bin/python}
LIM=${LIM:-8G}
mkdir -p "$W" results/dev
[ -f "$W/plain.memb.u32" ] || ./lim.sh $LIM $PY -I shape.py solve --store "$store" --work "$W"
for v in $variants; do
  case $v in
    *-up) m=${v%-up}; x="--up on --tag=-up" ;;
    *) m=$v; x="" ;;
  esac
  ./lim.sh $LIM $PY -I shape.py build --method $m $x --store "$store" --work "$W" ${EXTRA:-} > "$W/build_$v.json"
  [ -n "${NOMETRICS:-}" ] || ./lim.sh $LIM $PY -I shape.py metrics --tree "$W/tree_$v.npz" --store "$store" --graph $g --plain-json "$W/plain.json" --out results/dev/$g.$v.json
  [ -n "${NOEXPORT:-}" ] || $PY -I shape.py export --tree "$W/tree_$v.npz" --graph $g --out "$W/hier_$v" > /dev/null
done
