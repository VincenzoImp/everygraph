#!/usr/bin/env bash
# F2 headline numbers. Offline after the first setup (needs .venv + dist wheel built; see REPORT.md "Rebuild").
# Usage: ./bench.sh [quick]   (quick: skips lj NetworkX comparison, which needs ~12 GB RAM and ~4 min)
set -euo pipefail
D=$(cd "$(dirname "$0")" && pwd); cd "$D"
source .venv/bin/activate
export CARGO_HOME=$D/dl/cargo CARGO_TARGET_DIR=$D/rust/target UV_CACHE_DIR=$D/dl/uvcache UV_PYTHON_INSTALL_DIR=$D/dl/pythons UV_OFFLINE=1
LJ=/home/tux/graphlab/data/csr/lj
mkdir -p results
# 0. build (offline, uses vendored registry in dl/cargo)
(cd rust && cargo build --release --offline -p glabcore -q)
(cd rust/glabpy && maturin build --release --offline --features abi3 -o ../../dist-abi3 -q 2>/dev/null) || true
uv pip install -q --reinstall --offline dist-abi3/*.whl
[ -f data/lj/edges_und.csv ] || python scripts/prep.py lj
[ -f data/lj1m/edges_und.csv ] || python scripts/make_lj1m.py
ls -l dist-abi3/*.whl | awk '{print "{\"wheel_bytes\":"$5"}"}' > results/wheel.json
# 1. Goal B: binding overhead / zero-copy / GIL / python-vs-native, all 32 threads, whole machine
flock /home/tux/graphlab/bench.lock env GLAB_THREADS=32 python scripts/goal_b.py $LJ results/goalB_lj_t32.json > /dev/null
# 2. abi3 wheel under uv run on several CPythons (needs the interpreters already in dl/pythons)
W=$(ls $D/dist-abi3/*.whl)
for py in 3.10 3.12 3.14; do uv run -q --offline --no-project --python $py --with $W scripts/uv_smoke.py $LJ || echo "{\"python\":\"$py\",\"error\":true}"; done > results/uv_smoke.txt
# 3. Goal C: NetworkX backend on the 1M-edge graph (+ lj unless quick)
env RAYON_NUM_THREADS=16 python scripts/goal_c.py data/lj1m/edges_und.csv results/goalC_lj1m.json > /dev/null
if [ "${1:-}" != quick ]; then
  flock /home/tux/graphlab/bigmem.lock systemd-run --user --scope -q -p MemoryMax=30G -p MemorySwapMax=0 \
    env RAYON_NUM_THREADS=16 python scripts/goal_c.py data/lj/edges_und.csv results/goalC_lj.json > /dev/null
fi
# 4. Goal A: engine query bar (quick mode, see scripts/engines/run_all.sh)
[ -x scripts/engines/run_all.sh ] && QUICK=1 scripts/engines/run_all.sh || echo "engines runner missing"
echo "done: results/*.json"
