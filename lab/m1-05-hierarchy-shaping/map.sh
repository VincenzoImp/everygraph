#!/usr/bin/env bash
# Experiment #19 (m1-05): the visual check. Lab layout and tiles (lab/e-map/gmap) on a shaped
# hierarchy, then screenshots in the lab viewer (headless Chromium, server on 127.0.0.1 only,
# stopped by PID).
#   ./map.sh GRAPH METHOD CSR_DIR [PORT]      reads $S/GRAPH/hier_METHOD, writes $S/GRAPH/map_METHOD/
#                                             and screenshots to results/screens/GRAPH-METHOD-*.png
set -eu
cd "$(dirname "$0")"
HERE=$PWD
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
PW=${PW:-/home/tux/graphlab/scratch/pw}
PY=${PY:-$S/venv/bin/python}
g=$1; m=$2; csr=$3; port=${4:-8791}
LIM=${LIM:-16G}
GMAP=../e-map/gmap/target/release/gmap
W=$S/$g/map_$m
SHOTS=${SHOTS:-$S/screens}
rm -rf "$W"; mkdir -p "$W/www/out" "$W/pw" "$SHOTS"
GMAP_THREADS=${T:-16} ./lim.sh $LIM $GMAP layout --graph "$csr" --hier "$S/$g/hier_$m" --out "$W/layout" > "$W/layout.json" 2> "$W/layout.err"
GMAP_THREADS=${T:-16} ./lim.sh $LIM $GMAP tiles --graph "$csr" --layout "$W/layout" --out "$W/map.pmtiles" --json "$W/tiles.json" > "$W/tiles.out" 2> "$W/tiles.err"
$PY -I targets.py "$W/layout" > "$W/targets.json"
ln -s "$HERE/../e-map/viewer" "$W/www/viewer"
ln -s "$W/map.pmtiles" "$W/www/out/map.pmtiles"
ln -s "$PW/node_modules" "$W/pw/node_modules"
cp shots.mjs "$W/pw/"
setsid node ../e-map/viewer/serve.mjs "$W/www" "$port" > "$W/serve.out" 2>&1 &
spid=$!
trap 'kill -TERM -- -$spid 2>/dev/null || kill -TERM $spid 2>/dev/null || true' EXIT
for _ in $(seq 50); do grep -q 127.0.0.1 "$W/serve.out" 2>/dev/null && break; sleep 0.1; done
(cd "$W/pw" && node shots.mjs "http://127.0.0.1:$port/viewer/index.html?file=../out/map.pmtiles" "$W/targets.json" "$SHOTS/$g-$m") > "$W/shots.json"
python3 -I - "$W" <<'EOF'
import json, sys
W = sys.argv[1]
lay = json.load(open(W + "/layout.json"))
t = json.load(open(W + "/targets.json"))
print(json.dumps({"layout_s": lay["layout_s"], "virtual": lay["virtual"], "max_depth": lay["max_depth"], "tnodes_alive": lay["tnodes_alive"], **t["counts"], "shots": [s["name"] for s in json.load(open(W + "/shots.json"))["shots"]]}))
EOF
