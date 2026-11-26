#!/usr/bin/env bash
# m1-06 (#20): the lab maps of lj and orkut (lab/e-map/gmap on the hierarchies in $EVERYGRAPH_DATA/hier), then the
# overview analysis and the preview's data. Load-independent counts only; every step under an 8 GiB limit.
#   ./maps.sh      writes results/overview_{lj,orkut}.json, results/popping_{lj,orkut}.json and
#                  design/preview/style/viewer-assets/overview-{lj,orkut}.js
set -eu
cd "$(dirname "$0")"
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
S=${S:-/home/tux/graphlab/scratch/m1-06-edge-rendering}
PY=${PY:-$S/venv/bin/python}     # numpy
T=${T:-16}
lim() { systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 "$@"; }
mkdir -p "$S/maps" results
(cd ../e-map/gmap && CARGO_TARGET_DIR="$S/target" cargo build --release -j 6 2>&1 | tail -1)
G=$S/target/release/gmap
for g in lj orkut; do
  GMAP_THREADS=$T lim "$G" layout --graph "$D/csr/$g" --hier "$D/hier/$g" --out "$S/maps/$g-layout" > "$S/maps/$g-layout.json"
  GMAP_THREADS=$T lim "$G" tiles --graph "$D/csr/$g" --layout "$S/maps/$g-layout" --out "$S/maps/$g.pmtiles" --json "results/tiles_$g.json" > /dev/null
  lim "$PY" -I overview.py "$D/csr/$g" "$S/maps/$g-layout" "$g" --json "results/overview_$g.json" --hier "$D/hier/$g" \
    --preview "../../design/preview/style/viewer-assets/overview-$g.js"
  lim "$PY" -I popping.py "$S/maps/$g.pmtiles" "results/popping_$g.json" > /dev/null
done
