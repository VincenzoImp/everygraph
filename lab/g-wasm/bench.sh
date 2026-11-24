#!/bin/bash
# Reproduces the headline numbers of experiment G (wasm / browser tier).
# Writes results/*.jsonl. No network needed (private toolchain + Chromium in tools/).
# Prereq (once, needs network): see REPORT.md "Toolchain recipe" (tools/ install), then ./build_wasm.sh
# Usage: ./bench.sh [quick]      (quick = skip memory-ceiling probes)
set -uo pipefail
cd "$(dirname "$0")"
D=$PWD; CSR=/home/tux/graphlab/data/csr/lj; LOCK=/home/tux/graphlab/bench.lock
export PLAYWRIGHT_BROWSERS_PATH=$D/tools/pw-browsers
mkdir -p results data
[ -f data/lj.txt ] || zcat /home/tux/graphlab/data/raw/com-lj.ungraph.txt.gz > data/lj.txt
[ -f data/lj10.txt ] || head -n 3468123 data/lj.txt > data/lj10.txt     # 10x smaller prefix sample
[ -x target/release/gnative ] || { echo "build first: cargo build --release -p gnative; ./build_wasm.sh"; exit 1; }
NT=$(nproc)
R=results; : > $R/native.jsonl; : > $R/node.jsonl; : > $R/chromium.jsonl; : > $R/ceiling.jsonl

echo "== native (rayon), 1 thread and $NT threads"
flock $LOCK bash -c "
for t in 1 $NT; do
  for a in 'text data/lj10.txt' 'text data/lj.txt' 'text2 data/lj.txt' 'csr $CSR'; do
    RAYON_NUM_THREADS=\$t ./target/release/gnative \$a >> $R/native.jsonl
  done
done"

echo "== node (single-threaded wasm32 / wasm64), cold tiering (= browser default) and --no-liftoff (optimized tier)"
for v in st st-simd w64; do
  for fl in "" "--no-liftoff"; do
    for a in "text data/lj10.txt" "csr $CSR" "stream2 data/lj.txt"; do
      node $fl js/node_run.mjs $v $a | sed "s/^{/{\"node_flags\":\"$fl\",/" >> $R/node.jsonl
    done
  done
done

echo "== headless chromium: single-threaded variants, then wasm-bindgen-rayon thread scaling"
cd js
CFG='[
 {"variant":"st","mode":"text","ds":"lj10.txt"},{"variant":"st","mode":"csr","ds":"lj"},
 {"variant":"st-simd","mode":"csr","ds":"lj"},{"variant":"w64","mode":"csr","ds":"lj"},
 {"variant":"mt","mode":"csr","ds":"lj","threads":1},{"variant":"mt","mode":"csr","ds":"lj","threads":2},
 {"variant":"mt","mode":"csr","ds":"lj","threads":4},{"variant":"mt","mode":"csr","ds":"lj","threads":8},
 {"variant":"mt","mode":"csr","ds":"lj","threads":16},{"variant":"mt","mode":"csr","ds":"lj","threads":'$NT'},
 {"variant":"mt-simd","mode":"csr","ds":"lj","threads":'$NT'},
 {"variant":"mt","mode":"text","ds":"lj10.txt","threads":1},{"variant":"mt","mode":"text","ds":"lj10.txt","threads":'$NT'},
 {"variant":"mt","mode":"stream2","ds":"lj.txt","threads":1},{"variant":"mt","mode":"stream2","ds":"lj.txt","threads":'$NT'},
 {"variant":"mt","mode":"file2","path":"'$D'/data/lj.txt","threads":'$NT'},
 {"variant":"mt","mode":"text","ds":"lj.txt","threads":'$NT'}
]'
flock $LOCK node browser_run.mjs "$CFG" >> ../$R/chromium.jsonl
# same, optimized tier forced (shows the cost of V8 baseline-tier on one-shot kernels)
CHROME_ARGS="--js-flags=--no-liftoff" flock $LOCK node browser_run.mjs '[{"variant":"st","mode":"csr","ds":"lj"},{"variant":"mt","mode":"csr","ds":"lj","threads":1},{"variant":"mt","mode":"csr","ds":"lj","threads":'$NT'}]' | sed 's/^{/{"chrome_flags":"--js-flags=--no-liftoff",/' >> ../$R/chromium.jsonl

echo "== probes: WebGPU, OPFS, File read throughput"
flock $LOCK node probe_run.mjs $D/data/lj.txt > ../$R/probe.json
cd ..

if [ "${1:-}" != quick ]; then
  echo "== memory ceiling (synthetic lj-like graphs, n = m/8.7, 1 PR iteration + CC)"
  # wasm32 one-pass (edge list + CSR resident) and two-pass (edges streamed twice)
  CFG='[{"variant":"mt","mode":"gen","n":25862068,"m":225000000,"threads":16},
        {"variant":"mt","mode":"gen","n":28735632,"m":250000000,"threads":16},
        {"variant":"mt","mode":"gen2","n":30459770,"m":265000000,"threads":16},
        {"variant":"mt","mode":"gen2","n":31609195,"m":275000000,"threads":16}]'
  (cd js && flock $LOCK node browser_run.mjs "$CFG") >> $R/ceiling.jsonl
  # wasm64 in node: beyond 4 GiB (one-pass, ~5.5 GB)
  node js/node_run.mjs w64 gen 34482758 300000000 >> $R/ceiling.jsonl
fi
echo "done; results in $R/"
