#!/bin/bash
# m1-18 (#175): our map layout against published large-graph layouts, lj and orkut.
# Installs the reference tools without root into $S, converts the graphs, runs every layout
# under a hard memory limit, scores every layout with the same metrics.py, and writes
# results/runs/<layout>_<graph>.json (what ran, time, memory) and results/metrics/*.json.
#
#   ./run_all.sh [setup] [layouts] [metrics]          (default: all three)
#
# Threads: 8 for every multi-threaded CPU tool; FM3 and sfdp are single-threaded; FA2 runs
# on the GPU (cuGraph), as a reference only. Timings are exploratory unless the machine is idle.
set -uo pipefail
L="$(cd "$(dirname "$0")" && pwd)"
R=$L/results
S=${S:-/home/tux/graphlab/scratch/m1-18-layout-baseline}
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
T=${THREADS:-8}
E=$S/env/tools
PY="$E/bin/python -I"
RP="$S/env/rapids/bin/python -I"
GM=$S/target/release/gmap
STEPS=${*:-setup layouts metrics}
mkdir -p "$S"/{dl/gt,src,build,work,out,pos,logs} "$R"/{runs,metrics}

lim() { # lim GRAPH LIMIT NAME TAG=V... -- CMD: run CMD in a memory-limited scope, record time and memory
  local g=$1 limit=$2 name=$3; shift 3
  local tags=(); while [ "$1" != "--" ]; do tags+=(--tag "$1"); shift; done; shift
  $PY "$L/measure.py" --out "$R/runs/${name}_$g.json" --limit "$limit" --timeout "${TIMEOUT:-10800}" \
    --tag graph="$g" --tag threads="$T" "${tags[@]}" -- \
    systemd-run --user --scope -q -p MemoryMax="$limit" -p MemorySwapMax=0 nice -n 10 "$@" \
    > "$S/logs/${name}_$g.log" 2>&1 || echo "$name on $g: failed (see $R/runs/${name}_$g.json)"
}

if [[ $STEPS == *setup* ]]; then
  [ -x "$S/mamba/bin/micromamba" ] || (mkdir -p "$S/mamba" && cd "$S/mamba" &&
    curl -sSfL https://micro.mamba.pm/api/micromamba/linux-64/latest | tar -xj bin/micromamba)
  [ -x "$E/bin/sfdp" ] || MAMBA_ROOT_PREFIX=$S/mamba/root "$S/mamba/bin/micromamba" create -y -q -p "$E" \
    -c conda-forge graphviz=14.1.2 gsl libboost-devel python=3.12 numpy scipy scikit-learn
  [ -x "$S/env/rapids/bin/python" ] || { uv venv -q -p 3.12 "$S/env/rapids" &&
    uv pip install -q -p "$S/env/rapids/bin/python" --extra-index-url=https://pypi.nvidia.com cugraph-cu12==26.10.* numpy; }
  for gt in com-lj com-orkut; do
    [ -f "$S/dl/gt/$gt.top5000.cmty.txt.gz" ] || curl -sSf -o "$S/dl/gt/$gt.top5000.cmty.txt.gz" \
      "https://snap.stanford.edu/data/bigdata/communities/$gt.top5000.cmty.txt.gz"
  done
  [ -d "$S/src/DRGraph" ] || bash "$L/fetch_src.sh" ZJUVAI/DRGraph 4f3fda215986f0a1258c5da3f598afa626847103
  [ -d "$S/src/BatchLayout" ] || bash "$L/fetch_src.sh" khaled-rahman/BatchLayout d2e260d82c0fe8c093232d12b61463d025ddcffb
  [ -d "$S/src/ogdf" ] || bash "$L/fetch_src.sh" ogdf/ogdf foxglove-202510
  # DRGraph: Boost and GSL from the conda env; shared Boost instead of static
  sed -i 's/set(Boost_USE_STATIC_LIBS ON)/set(Boost_USE_STATIC_LIBS OFF)/' "$S/src/DRGraph/src/CMakeLists.txt"
  mkdir -p "$S/build/drgraph" && (cd "$S/build/drgraph" &&
    cmake -DCMAKE_PREFIX_PATH="$E" -DCMAKE_CXX_FLAGS="-I$E/include" -DCMAKE_C_FLAGS="-I$E/include" \
      -DCMAKE_EXE_LINKER_FLAGS="-L$E/lib -Wl,-rpath,$E/lib" "$S/src/DRGraph" >/dev/null && make -j6 >/dev/null)
  # BatchLayout: its own makefile on a clean copy (the tarball ships a prebuilt binary)
  rm -rf "$S/build/batchlayout" && cp -r "$S/src/BatchLayout/BatchLayoutCode" "$S/build/batchlayout" &&
    rm -f "$S/build/batchlayout/bin/BatchLayout" && make -C "$S/build/batchlayout" >/dev/null 2>&1
  # OGDF: static library, then the FM3 driver
  mkdir -p "$S/build/ogdf" && (cd "$S/build/ogdf" &&
    cmake -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DOGDF_WARNING_ERRORS=OFF "$S/src/ogdf" >/dev/null &&
    make -j6 OGDF >/dev/null)
  g++ -O2 -std=c++17 -I"$S/src/ogdf/include" -I"$S/build/ogdf/include/ogdf-release" "$L/ogdf_fmmm.cpp" \
    -o "$S/build/ogdf_fmmm" "$S/build/ogdf/libOGDF.a" "$S/build/ogdf/libCOIN.a" -pthread
  (cd "$L/../e-map/gmap" && CARGO_TARGET_DIR=$S/target cargo build --release -q)
  [ -f "$S/work/lj/graph.dot" ] || systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 \
    $PY "$L/convert.py" "$D/csr/lj" "$S/work/lj"
  [ -f "$S/work/orkut/edges.txt" ] || systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 \
    $PY "$L/convert.py" "$D/csr/orkut" "$S/work/orkut" edges
fi

if [[ $STEPS == *layouts* ]]; then
  for g in lj orkut; do
    n=$(python3 -c "import json;print(json.load(open('$D/csr/$g/meta.json'))['n'])")
    for v in "hcl:" "hcls:--solve stress"; do
      nm=${v%%:*}; opt=${v#*:}
      lim $g 8G $nm tool="gmap layout $opt" -- env GMAP_THREADS=$T "$GM" layout --graph "$D/csr/$g" \
        --hier "$D/hier/$g" --out "$S/out/${nm}_$g" --phi 0.72 $opt
      cp "$S/out/${nm}_$g/pos.f32" "$S/pos/${nm}_$g.f32"
    done
    $PY "$L/topos.py" random - "$n" "$S/pos/random_$g.f32"
    lim $g 12G fa2 tool="cuGraph force_atlas2 (GPU)" threads=GPU options="cuGraph defaults, max_iter=500" -- \
      $RP "$L/fa2_cugraph.py" "$D/csr/$g" "$S/pos/fa2_$g.f32" 500
    lim $g $([ $g = lj ] && echo 8G || echo 24G) drgraph tool=DRGraph version=ZJUVAI/DRGraph@4f3fda2 options="-neg 5 -samples 400 -gamma 0.1 -mode 1 -A 2 -B 1 (README)" -- \
      "$S/src/DRGraph/Vis" -input "$S/work/$g/edges.txt" -output "$S/out/drgraph_$g.txt" \
      -neg 5 -samples 400 -gamma 0.1 -mode 1 -A 2 -B 1 -threads $T
    [ -f "$S/out/drgraph_$g.txt" ] && $PY "$L/topos.py" drgraph "$S/out/drgraph_$g.txt" "$n" "$S/pos/drgraph_$g.f32"
  done
  # diagnostic: our layout with the top tree levels started from DRGraph's community centroids
  for d in 1 3; do
    lim lj 8G hclg$d tool="gmap layout --seed-pos drgraph --seed-depth $d" -- env GMAP_THREADS=$T "$GM" layout \
      --graph "$D/csr/lj" --hier "$D/hier/lj" --out "$S/out/hclg${d}_lj" --phi 0.72 \
      --seed-pos "$S/pos/drgraph_lj.f32" --seed-depth $d
    cp "$S/out/hclg${d}_lj/pos.f32" "$S/pos/hclg${d}_lj.f32"
  done
  # lj only: the tools that did not fit or did not finish there are not tried on orkut
  lim lj 24G fm3 tool="OGDF FMMMLayout" version=ogdf/ogdf@foxglove-202510 threads=1 \
    options="useHighLevelOptions, BeautifulAndFast" -- "$S/build/ogdf_fmmm" "$S/work/lj/edges.txt" "$S/pos/fm3_lj.f32" 1
  lim lj 24G sfdp tool=sfdp version="graphviz 14.1.2 (conda-forge)" threads=1 \
    options="-Goverlap=true -Gsplines=false -Tplain" -- \
    "$E/bin/sfdp" -Goverlap=true -Gsplines=false -Tplain -o "$S/out/sfdp_lj.plain" "$S/work/lj/graph.dot"
  [ -f "$S/out/sfdp_lj.plain" ] && $PY "$L/topos.py" plain "$S/out/sfdp_lj.plain" 3997962 "$S/pos/sfdp_lj.f32"
  # -init 0 (greedy, the default) is killed at 8G and 16G; -init 1 is random
  mkdir -p "$S/out/bl"
  lim lj 16G batchlayout tool=BatchLayout version=khaled-rahman/BatchLayout@d2e260d \
    options="-algo 4 (Barnes-Hut) -iter 600 -init 1 (random)" -- bash -c "ulimit -s 4000000; exec \
    $S/build/batchlayout/bin/BatchLayout -input $S/work/lj/graph.mtx -output $S/out/bl/ -iter 600 -threads $T -algo 4 -init 1"
  f=$(ls "$S"/out/bl/*.txt 2>/dev/null | head -1)
  [ -n "$f" ] && $PY "$L/topos.py" batchlayout "$f" 3997962 "$S/pos/batchlayout_lj.f32"
fi

if [[ $STEPS == *metrics* ]]; then
  for p in "$S"/pos/*.f32; do
    b=$(basename "$p" .f32); nm=${b%_*}; g=${b##*_}
    OMP_NUM_THREADS=$T systemd-run --user --scope -q -p MemoryMax=12G -p MemorySwapMax=0 nice -n 10 \
      $PY "$L/metrics.py" --graph "$D/csr/$g" --hier "$D/hier/$g" --gt "$S/dl/gt/com-$g.top5000.cmty.txt.gz" \
      --pos "$p" --name "$nm" --out "$R/metrics/${nm}_$g.json" > "$S/logs/metrics_${nm}_$g.log" 2>&1 ||
      echo "metrics $nm on $g failed"
  done
  for g in lj orkut; do
    args=(); for nm in hcl hcls hclg1 drgraph fa2 batchlayout random; do
      [ -f "$S/pos/${nm}_$g.f32" ] && args+=("$nm=$S/pos/${nm}_$g.f32"); done
    systemd-run --user --scope -q -p MemoryMax=12G -p MemorySwapMax=0 nice -n 10 \
      $PY "$L/crosslevel.py" "$D/csr/$g" "$D/hier/$g" "$R/crosslevel_$g.json" "${args[@]}" > /dev/null
  done
  $PY "$L/table.py" "$R" > "$R/table.md"
fi
