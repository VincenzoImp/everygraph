#!/bin/bash
# Baselines: the bar to beat. Run on an idle machine. Writes results/baselines.jsonl and results/*.log
cd /home/tux/graphlab/exp/a-baselines; rm -f results/baselines.jsonl
PY=.venv/bin/python
for g in lj orkut; do
  $PY bench_lib.py networkit $g 32; $PY bench_lib.py networkit $g 1
  $PY bench_lib.py igraph $g 1; $PY bench_lib.py scipy $g 1
  $PY duck_ingest.py $g 32
  for a in louvain leiden; do for t in 32 1; do ./gve/gve-$a-t$t data/$g.mtx 1 0 > results/gve-$a-$g-t$t.log 2>&1; done; done
done
$PY bench_lib.py rustworkx lj 1
./run_largegraph.sh
