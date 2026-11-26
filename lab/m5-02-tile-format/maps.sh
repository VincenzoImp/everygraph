#!/usr/bin/env bash
# m5-02 (#68): the lab maps of lj and orkut that the payload comparison reads (lab/e-map/gmap on the
# hierarchies in $EVERYGRAPH_DATA/hier), as lab/m1-06-edge-rendering/maps.sh builds them. Every step
# under an 8 GiB limit.
set -eu
cd "$(dirname "$0")"
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
S=${S:-/home/tux/graphlab/scratch/m5-02-tile-format}
T=${T:-8}
mkdir -p "$S/maps"
lim() { systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 "$@"; }
(cd ../e-map/gmap && CARGO_TARGET_DIR="$S/target" cargo build --release -j 6 2>&1 | tail -1)
G=$S/target/release/gmap
for g in lj orkut; do
  GMAP_THREADS=$T lim "$G" layout --graph "$D/csr/$g" --hier "$D/hier/$g" --out "$S/maps/$g-layout" > "$S/maps/$g-layout.json"
  GMAP_THREADS=$T lim "$G" tiles --graph "$D/csr/$g" --layout "$S/maps/$g-layout" --out "$S/maps/$g.pmtiles" --json "$S/maps/tiles_$g.json" > /dev/null
done
echo done
