#!/bin/bash
# m1-07 (#21): layout area and the global gap, lj and orkut.
#
#   ./run_all.sh [build] [layouts] [determinism] [metrics] [tiles]   (default: all)
#
# build        gmap (lab/e-map/gmap) into $S/target; a Python env with numpy, scipy, scikit-learn
# layouts      hcl hcls hclsb cells cellsb at 8 threads (layouts.sh), 8G each
# determinism  the same at 1 thread; pos.f32, tnodes.bin, order.u32 compared byte for byte
# metrics      m1-18's metrics.py and crosslevel.py, unchanged, plus gap.py; 12G each
# tiles        gmap tiles for hcl, hcls and cells (tile count, file size, zmax)
# Every time and memory figure is exploratory: the machine is shared.
set -uo pipefail
L="$(cd "$(dirname "$0")" && pwd)"
R=$L/results
B=$L/../m1-18-layout-baseline
export S=${S:-/home/tux/graphlab/scratch/m1-07-layout-area}
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
PY="$S/env/bin/python -I"
GT=$S/gt
V=${VARIANTS:-hcl hcls hclsb cells cellsb}
G=${GRAPHS:-lj orkut}
STEPS=${*:-build layouts determinism metrics tiles}
mkdir -p "$S"/{pos,out,logs} "$GT" "$R"/{layout,metrics,tiles}
lim() { systemd-run --user --scope -q -p MemoryMax="$1" -p MemorySwapMax=0 nice -n 10 "${@:2}"; }

if [[ $STEPS == *build* ]]; then
  (cd "$L/../e-map/gmap" && CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR=$S/target cargo build --release -q)
  [ -x "$S/env/bin/python" ] || { uv venv -q -p 3.12 "$S/env" && uv pip install -q -p "$S/env/bin/python" numpy scipy scikit-learn; }
  for g in $G; do
    [ -f "$GT/com-$g.top5000.cmty.txt.gz" ] || curl -sSf -o "$GT/com-$g.top5000.cmty.txt.gz" \
      "https://snap.stanford.edu/data/bigdata/communities/com-$g.top5000.cmty.txt.gz"
  done
fi

if [[ $STEPS == *layouts* ]]; then
  for g in $G; do
    # shellcheck disable=SC2086
    "$L/layouts.sh" $g 8 $V
    for v in $V; do cp "$S/out/${v}_${g}_t8/layout.json" "$R/layout/${v}_$g.json"; done
  done
fi

if [[ $STEPS == *determinism* ]]; then
  : > "$R/determinism.txt"
  for g in $G; do
    # shellcheck disable=SC2086
    "$L/layouts.sh" $g 1 $V
    for v in $V; do for f in pos.f32 tnodes.bin order.u32; do
      if cmp -s "$S/out/${v}_${g}_t1/$f" "$S/out/${v}_${g}_t8/$f"; then r=identical; else r=DIFFERENT; fi
      echo "$g $v $f 1 vs 8 threads: $r $(sha256sum < "$S/out/${v}_${g}_t8/$f" | cut -c1-16)" >> "$R/determinism.txt"
    done; done
    for v in $V; do python3 -c "import json;print('$g $v t1', json.load(open('$S/out/${v}_${g}_t1/layout.json'))['layout_s'])"; done >> "$R/determinism.txt"
  done
fi

if [[ $STEPS == *metrics* ]]; then
  for g in $G; do
    for v in $V; do
      OMP_NUM_THREADS=8 lim 12G $PY "$B/metrics.py" --graph "$D/csr/$g" --hier "$D/hier/$g" \
        --gt "$GT/com-$g.top5000.cmty.txt.gz" --pos "$S/pos/${v}_$g.f32" --name "$v" \
        --out "$R/metrics/${v}_$g.json" > "$S/logs/metrics_${v}_$g.log" 2>&1 || echo "metrics $v $g failed"
    done
    args=(); for v in $V; do args+=("$v=$S/pos/${v}_$g.f32"); done
    lim 12G $PY "$B/crosslevel.py" "$D/csr/$g" "$D/hier/$g" "$R/crosslevel_$g.json" "${args[@]}" > /dev/null
    lim 12G $PY "$L/gap.py" "$D/csr/$g" "$D/hier/$g" 6 "$R/gap_$g.json" "${args[@]}" > /dev/null
  done
fi

if [[ $STEPS == *tiles* ]]; then
  for g in $G; do
    for v in hcl hcls cells; do
      lim 8G env GMAP_THREADS=8 "$S/target/release/gmap" tiles --graph "$D/csr/$g" --layout "$S/out/${v}_${g}_t8" \
        --out "$S/out/tiles_${v}_$g.pmtiles" > "$R/tiles/${v}_$g.json" 2> "$S/logs/tiles_${v}_$g.err" || echo "tiles $v $g failed"
    done
  done
fi
