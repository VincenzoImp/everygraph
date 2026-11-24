#!/bin/sh
# Current largegraph prototype on lj and orkut (CSV with header).
cd /home/tux/graphlab/exp/a-baselines
for g in lj orkut; do
  [ -f data/$g.csv ] || { echo source,target > data/$g.csv; zcat /home/tux/graphlab/data/raw/com-$g.ungraph.txt.gz | grep -v '^#' | tr '\t' ',' >> data/$g.csv; }
  rm -rf lg/$g
  python3 /home/tux/graphlab/tools/timev.py /home/tux/projects/largegraph/.venv/bin/largegraph analyze data/$g.csv --output lg/$g --ram 16GB > results/largegraph-$g.log 2>&1
done
