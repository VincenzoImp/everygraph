#!/usr/bin/env bash
# Goal A: embedded graph query engines on lj. Usage: [QUICK=1] [NEO4J=1] scripts/engines/run_all.sh
# QUICK=1 -> fewer query inputs (~5-8 min total). NEO4J=1 -> also offline import + server (127.0.0.1) + GDS.
# Needs: .venv (ladybug, polars, graphistry, igraph, rustworkx, neo4j), .venv-duck (duckdb 1.3.2 + duckpgq
# cached in dl/home/.duckdb), prepared data in data/lj (scripts/prep.py lj + edges_und.txt). No network needed
# once extensions/tarballs are cached.
set -euo pipefail
R=/home/tux/graphlab/exp/f2-python; cd "$R"
export QUICK=${QUICK:-0} HOME_SAVE=$HOME
CORES=${CORES:-0-2}
cap() { systemd-run --user --scope -q -p MemoryMax=${MEM:-6G} -p MemorySwapMax=0 taskset -c $CORES "$@"; }
PY=$R/.venv/bin/python; PYD=$R/.venv-duck/bin/python
[ -f data/lj/edges_und.parquet ] || $PY scripts/prep.py lj
[ -f data/lj/edges_und.txt ] || $PY -c "import pyarrow.parquet as pq, pyarrow.csv as pc; pc.write_csv(pq.read_table('data/lj/edges_und.parquet'),'data/lj/edges_und.txt',pc.WriteOptions(include_header=False,delimiter=' '))"
[ -f results/queries_lj.json ] || $PY scripts/engines/make_queries.py lj
cd scripts/engines
echo "== igraph";    cap $PY -W ignore eng_igraph.py lj    > $R/results/log_igraph.txt 2>&1 || echo "  FAILED (see results/log_igraph.txt)"
echo "== rustworkx"; cap $PY -W ignore eng_rustworkx.py lj > $R/results/log_rustworkx.txt 2>&1 || echo "  FAILED (see results/log_rustworkx.txt)"
echo "== ladybug";   HOME=$R/dl/home cap $PY -W ignore eng_ladybug.py lj > $R/results/log_ladybug.txt 2>&1 || echo "  FAILED (see results/log_ladybug.txt)"
echo "== gfql";      cap $PY -W ignore eng_gfql.py lj      > $R/results/log_gfql.txt 2>&1 || echo "  FAILED (see results/log_gfql.txt)"
echo "== duckpgq";   HOME=$R/dl/home cap $PYD -W ignore eng_duckpgq.py lj > $R/results/log_duckpgq.txt 2>&1 || echo "  FAILED (see results/log_duckpgq.txt)"
if [ "${NEO4J:-0}" = 1 ]; then
  echo "== neo4j"
  export NEO4J_HOME=$R/dl/neo4j/neo4j-community-5.26.0; D=$R/data/lj
  [ -f $D/nodes_nohdr.txt ] || tail -n +2 $D/nodes.csv > $D/nodes_nohdr.txt
  echo "id:ID" > $D/neo_nodes_hdr.csv; echo ":START_ID :END_ID" > $D/neo_rels_hdr.csv
  $NEO4J_HOME/bin/neo4j stop >/dev/null 2>&1 || true
  s=$(date +%s.%N)
  cap $NEO4J_HOME/bin/neo4j-admin database import full neo4j --overwrite-destination --id-type=INTEGER --delimiter=' ' \
     --threads=3 --max-off-heap-memory=2g --nodes=N=$D/neo_nodes_hdr.csv,$D/nodes_nohdr.txt \
     --relationships=E=$D/neo_rels_hdr.csv,$D/edges_und.txt > $R/results/log_neo4j_import.txt 2>&1
  imp=$($PY -c "import time;print(time.time()-$s)")
  cap $NEO4J_HOME/bin/neo4j start
  for i in $(seq 1 60); do ss -ltn | grep -q 127.0.0.1:17687 && break; sleep 2; done; sleep 5
  pid=$(cat $NEO4J_HOME/run/neo4j.pid)
  MEM=3G cap $PY -W ignore eng_neo4j.py lj $imp $(du -sm $NEO4J_HOME/data/databases/neo4j | cut -f1) $pid > $R/results/log_neo4j.txt 2>&1 || true
  $NEO4J_HOME/bin/neo4j stop
fi
$PY summ.py $R/results/goalA_*lj*.json > $R/results/goalA_summary.txt 2>&1 || true
$PY table.py > $R/results/goalA_table.md || true
echo done
