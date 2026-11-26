#!/usr/bin/env bash
# m1-12 (#26) and m1-06 (#20): browser measurements, exploratory. Needs the lab maps built by lab/m1-06-edge-rendering/maps.sh
# and Playwright with Chromium and Firefox (PW=directory holding node_modules/playwright).
#   ./run.sh            writes results/viewer_<graph>_<config>.json and ../m1-06-edge-rendering/results/preview_<config>.json
# VIEWER_CONFIGS and FOURCPU_GRAPHS narrow a re-run. The range server binds 127.0.0.1 only and is stopped by its PID.
set -eu
cd "$(dirname "$0")"
HERE=$PWD
S=${S:-/home/tux/graphlab/scratch/m1-06-edge-rendering}
PW=${PW:-/home/tux/graphlab/scratch/pw}
PY=${PY:-$S/venv/bin/python}
PORT=${PORT:-8797}
RUN=$S/run
mkdir -p "$RUN" "$S/www/out" results ../m1-06-edge-rendering/results
ln -sfn "$PW/node_modules" "$RUN/node_modules"
cp ./*.mjs ../m1-06-edge-rendering/measure_preview.mjs "$RUN/"
ln -sfn "$HERE/../e-map/viewer" "$S/www/viewer"
for g in lj orkut; do
  ln -sfn "$S/maps/$g.pmtiles" "$S/www/out/$g.pmtiles"
  "$PY" -I target.py "$S/maps/$g-layout" > "$S/target_$g.json"
done
nice -n 10 node ../e-map/viewer/serve.mjs "$S/www" "$PORT" > "$S/serve.log" 2>&1 &
spid=$!
trap 'kill -TERM "$spid" 2>/dev/null || true' EXIT
for _ in $(seq 50); do grep -q 127.0.0.1 "$S/serve.log" 2>/dev/null && break; sleep 0.1; done
V=http://127.0.0.1:$PORT/viewer/index.html
(cd "$RUN" && node probe.mjs "http://127.0.0.1:$PORT/out/lj.pmtiles" "$HERE/results/probe.json" > /dev/null)
(cd "$RUN" && node firefox_webgl.mjs > "$HERE/results/firefox_webgl.jsonl")
uptime > results/load.txt
for g in lj orkut; do
  for c in ${VIEWER_CONFIGS:-chromium-gpu chromium-swiftshader firefox}; do
    (cd "$RUN" && node measure_viewer.mjs "$V" "$g" "$c" "$S/target_$g.json" "$HERE/results/viewer_${g}_${c}.json" > "$S/viewer_${g}_${c}.log" 2>&1) || echo "viewer $g $c failed"
  done
  # SwiftShader held to 4 logical CPUs: a rough stand-in for a laptop without a usable GPU (the cores are shared)
  case " ${FOURCPU_GRAPHS:-lj orkut} " in *" $g "*) ;; *) continue ;; esac
  (cd "$RUN" && taskset -c 0-3 node measure_viewer.mjs "$V" "$g" chromium-swiftshader "$S/target_$g.json" "$HERE/results/viewer_${g}_chromium-swiftshader-4cpu.json" > "$S/viewer_${g}_4cpu.log" 2>&1) || echo "viewer $g 4cpu failed"
done
for c in chromium-gpu chromium-swiftshader firefox; do
  (cd "$RUN" && node measure_preview.mjs "file://$HERE/../../design/preview/style/overview.html" "$c" "$HERE/../m1-06-edge-rendering/results/preview_${c}.json" > "$S/preview_${c}.log" 2>&1) || echo "preview $c failed"
done
uptime >> results/load.txt
