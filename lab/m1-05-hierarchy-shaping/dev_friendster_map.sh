#!/usr/bin/env bash
# Exploratory: does the lab layout place Friendster's rec hierarchy under 16 GiB (graph
# memory-mapped, as the tracer's 16 GiB layout)? Then an SVG of its top levels (no tiles: the
# lab tile builder does not fit 16 GiB, FINDINGS § M1-01).
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
PY=${PY:-$S/venv/bin/python}
W=$S/friendster
m=${1:-rec}
./dev_friendster.sh export-$m > /dev/null
rm -rf "$W/layout_$m"
GMAP_LOAD=mmap GMAP_THREADS=16 ./lim.sh 16G ../e-map/gmap/target/release/gmap layout --graph /home/tux/graphlab/data/csr/friendster --hier "$W/hier_$m" --out "$W/layout_$m" > "$W/layout_$m.json" 2> "$W/layout_$m.err"
python3 -I -c "import json,sys; d=json.load(open(sys.argv[1])); print(json.dumps({k: d[k] for k in ['layout_s','total_s','virtual','max_depth','tnodes_alive','peak_rss_mb','hier_levels']}))" "$W/layout_$m.json"
$PY -I targets.py "$W/layout_$m" > "$W/targets_$m.json"
$PY -I svgmap.py "$W/layout_$m" "$S/friendster-$m-top.svg" --depth 2 --title "friendster, $m: depth 1-2 (lab layout, 16 GiB)"
./svgshot.sh "$S/friendster-$m-top.svg" "results/screens/friendster-$m-1-top.png"
# inside the largest region of depth 1
read cx cy half < <($PY -I -c "import json,sys; t=[x for x in json.load(open(sys.argv[1]))['targets'] if x['name']=='3-largest-community'][0]; import numpy as np; print(t['x'], t['y'], 380/(256*2**t['z'])/2*1.15)" "$W/targets_$m.json")
$PY -I svgmap.py "$W/layout_$m" "$S/friendster-$m-largest.svg" --depth 4 --cx $cx --cy $cy --half $half --title "friendster, $m: inside the largest community, depth 1-4"
./svgshot.sh "$S/friendster-$m-largest.svg" "results/screens/friendster-$m-2-largest.png"
