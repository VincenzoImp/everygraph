#!/usr/bin/env bash
# Leiden hierarchies for the ordering bake-off (m1-09, #23), with the lab Leiden of b2-leiden.
#   ./leiden.sh <graph> <natural|cap64> <MemoryMax>
# Writes $S/hier/<graph>-<variant>/ (level_k.u32 parent maps) and results/leiden/<graph>-<variant>.json
# (time and modularity). Exploratory: the machine is shared.
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-09-hierarchy-order}
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
B=${B2:-$S/b2-target/release/b2}
g=$1; v=$2; lim=$3
case $v in
  natural) flags="--min-ratio 3" ;;
  cap64) flags="--cap 64 --max-passes 20 --min-ratio 3" ;;
  *) echo "variant: natural|cap64" >&2; exit 2 ;;
esac
mkdir -p "$S/hier" results/leiden
[ -e "$S/hier/$g-$v/hier.json" ] && exit 0
systemd-run --user --scope -q -p MemoryMax="$lim" -p MemorySwapMax=0 \
  "$B" run --graph "$D/csr/$g" --algo leiden --threads 16 $flags --lowmem --no-quot \
  --out-hier "$S/hier/$g-$v" --json "results/leiden/$g-$v.json" > "$S/leiden-$g-$v.log" 2>&1
