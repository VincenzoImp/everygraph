#!/bin/bash
# layouts.sh GRAPH THREADS NAME... : run the named layout variants of lab/e-map/gmap on GRAPH
# (lj or orkut, hierarchy data/hier/GRAPH) under an 8G limit; positions to $S/pos, the
# layout's JSON (time, memory, area per depth) to $S/out/<name>_<graph>_t<threads>/layout.json.
set -euo pipefail
S=${S:-/home/tux/graphlab/scratch/m1-07-layout-area}
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
GM=$S/target/release/gmap
g=$1 t=$2; shift 2
opts() {
  case $1 in
    hcl) echo "" ;;                                   # current packing, force solve
    hcls) echo "--solve stress" ;;                    # packing + stress solve (m1-18)
    hclsb) echo "--solve stress --boundary 1" ;;      # + boundary-aware placement (b)
    hclsb3) echo "--solve stress --boundary 3" ;;
    hclsb10) echo "--solve stress --boundary 10" ;;
    hclsb100) echo "--solve stress --boundary 100" ;;
    cells) echo "--solve stress --cells" ;;           # power-diagram cells (a), 20 Lloyd steps
    cells5) echo "--solve stress --cells --lloyd 5" ;;
    cellsd) echo "--solve stress --cells-in-disc" ;;      # cells inside each parent bubble
    cellsb) echo "--solve stress --boundary 1 --cells" ;;
    cellsb3) echo "--solve stress --boundary 3 --cells" ;;
    *) echo "unknown variant $1" >&2; exit 2 ;;
  esac
}
mkdir -p "$S"/{pos,out,logs}
for nm in "$@"; do
  o=$S/out/${nm}_${g}_t$t
  # shellcheck disable=SC2046
  systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 env GMAP_THREADS="$t" \
    "$GM" layout --graph "$D/csr/$g" --hier "$D/hier/$g" --out "$o" --phi 0.72 $(opts "$nm") \
    > /dev/null 2> "$S/logs/${nm}_${g}_t$t.err"
  [ "$t" = 8 ] && cp "$o/pos.f32" "$S/pos/${nm}_$g.f32"
  python3 -c "import json;j=json.load(open('$o/layout.json'));print('$nm $g t$t', j['layout_s'], 's', j['peak_rss_mb'], 'MB')"
done
