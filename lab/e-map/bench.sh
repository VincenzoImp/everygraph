#!/bin/bash
# Reproduces the headline numbers of experiment E (graph map). Writes results/*.json.
# Usage: ./bench.sh            (whole machine; wrap in `flock /home/tux/graphlab/bench.lock` when the lab is shared)
#        THREADS="1 4" ./bench.sh   to restrict the thread sweep
set -euo pipefail
E="$(cd "$(dirname "$0")" && pwd)"; cd "$E"
D=/home/tux/graphlab/data
BIN=$E/gmap/target/release/gmap
(cd gmap && nice -n 10 cargo build --release -j 6 2>&1 | tail -1)
mkdir -p results out
NT=$(nproc)
hier_of() { if [ -f "$D/hier/$1/hier.json" ]; then echo "$D/hier/$1"; else echo "$E/hier/$1"; fi; }
for g in lj orkut; do
  H=$(hier_of $g)
  [ -f "$H/hier.json" ] || { echo "no hierarchy for $g, skipping"; continue; }
  echo "== $g (hierarchy: $H)"
  # layout thread sweep
  for t in ${THREADS:-1 4 8 16 $NT}; do
    GMAP_THREADS=$t $BIN layout --graph $D/csr/$g --hier $H --out out/$g-hcl --phi 0.72 2>/dev/null > results/layout_${g}_t${t}.json
    python3 -c "import json;j=json.load(open('results/layout_${g}_t${t}.json'));print('layout $g threads',j['threads'],'layout_s',j['layout_s'],'total_s',j['total_s'],'rss_mb',j['peak_rss_mb'])"
  done
  # determinism: same seed, different thread count -> identical bytes
  GMAP_THREADS=3 $BIN layout --graph $D/csr/$g --hier $H --out out/$g-hcl-det --phi 0.72 >/dev/null 2>&1
  if cmp -s out/$g-hcl/pos.f32 out/$g-hcl-det/pos.f32 && cmp -s out/$g-hcl/tnodes.bin out/$g-hcl-det/tnodes.bin; then DET=true; else DET=false; fi
  echo "{\"graph\":\"$g\",\"deterministic_across_thread_counts\":$DET,\"sha256_pos\":\"$(sha256sum out/$g-hcl/pos.f32 | cut -d' ' -f1)\"}" > results/determinism_$g.json; cat results/determinism_$g.json
  rm -rf out/$g-hcl-det
  # quality metric + baselines (lj only for the baselines)
  GMAP_THREADS=$NT $BIN eval --graph $D/csr/$g --layout out/$g-hcl --json results/eval_${g}_hcl.json
  if [ $g = lj ]; then
    GMAP_THREADS=$NT $BIN embed --graph $D/csr/$g --out out/$g-fastrp --top out/$g-hcl/top.u32 > results/layout_${g}_fastrp.json; cat results/layout_${g}_fastrp.json
    GMAP_THREADS=$NT $BIN eval --graph $D/csr/$g --layout out/$g-fastrp --json results/eval_${g}_fastrp.json
    GMAP_THREADS=$NT $BIN layout --graph $D/csr/$g --hier $H --out out/$g-hcl-noext --phi 0.72 --no-ext 2>/dev/null > results/layout_${g}_noext.json
    GMAP_THREADS=$NT $BIN eval --graph $D/csr/$g --layout out/$g-hcl-noext --json results/eval_${g}_hcl_noext.json
    rm -rf out/$g-hcl-noext
  fi
  # tile pyramid + single-file container
  SHAS=""
  for t in 4 $NT; do
    GMAP_THREADS=$t $BIN tiles --graph $D/csr/$g --layout out/$g-hcl --out out/$g.pmtiles --json results/tiles_${g}_t${t}.json 2>/dev/null >/dev/null
    SHAS="$SHAS $(sha256sum out/$g.pmtiles | cut -d' ' -f1)"
    python3 -c "import json;j=json.load(open('results/tiles_${g}_t${t}.json'));print('tiles $g threads',j['threads'],'build_s',j['build_s'],'total_s',j['total_s'],'tiles',j['tiles'],'file_MB',round(j['file_bytes']/1e6,1),'median/max tile gz',j['tile_gz_median'],j['tile_gz_max'],'rss_mb',j['peak_rss_mb'])"
  done
  set -- $SHAS
  if [ "$1" = "$2" ]; then TDET=true; else TDET=false; fi
  echo "{\"graph\":\"$g\",\"tile_file_identical_across_thread_counts\":$TDET,\"sha256_pmtiles\":\"$2\"}" > results/determinism_tiles_$g.json; cat results/determinism_tiles_$g.json
done
echo "done; see results/*.json (viewer numbers: cd viewer && node serve.mjs .. 8765 & node capture.mjs lj ../img/viewer-lj ../results/viewer_lj.json ../out/lj-hcl/target.json)"
