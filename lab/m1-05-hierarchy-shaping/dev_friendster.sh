#!/usr/bin/env bash
# Exploratory Friendster runs under a hard 16 GiB limit (shared machine: no timing is quoted;
# official runs go through bench/specs/hierarchy-shaping.toml).
#   ./dev_friendster.sh solve|rec|capin|metrics-rec|metrics-capin|export-rec
# Each step runs in its own scope; the driver and every m102 it starts share the limit.
# m102 runs out of core with a 10 GB accountant budget (the driver keeps < 1 GB between rounds).
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
PY=${PY:-$S/venv/bin/python}
ST=/home/tux/graphlab/data/store-m113/friendster
W=$S/friendster
LIM=${LIM:-16G}
mkdir -p "$W" results/dev
common="--store $ST --work $W --ooc --budget-gb ${BUDGET:-10} --threads 16"
case $1 in
  solve) ./lim.sh $LIM $PY -I shape.py solve $common --no-hier ;;
  rec|rec-up|capin) ./lim.sh $LIM $PY -I shape.py build --method $1 $common ${EXTRA:-} --tag="${TAG:-}" > "$W/build_$1${TAG:-}.json" ;;
  metrics-*) m=${1#metrics-}; ./lim.sh ${MLIM:-24G} $PY -I shape.py metrics --tree "$W/tree_$m.npz" --store $ST --graph friendster --plain-json "$W/plain.json" --out results/dev/friendster.$m.json ;;
  export-*) m=${1#export-}; ./lim.sh 8G $PY -I shape.py export --tree "$W/tree_$m.npz" --graph friendster --out "$W/hier_$m" ;;
esac
