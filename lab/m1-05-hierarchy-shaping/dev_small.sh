#!/usr/bin/env bash
# Every exploratory record of the small graphs, from scratch (shared machine: quality numbers
# only; they are deterministic). Writes results/dev/*.json, results/capfix_lj.jsonl,
# results/determinism.jsonl, results/screens/*.png. About 25 minutes.
set -eu
cd "$(dirname "$0")"
D=/home/tux/graphlab/data
./dev_capfix.sh > results/capfix_lj.jsonl
for gs in lj:store-m113 orkut:store-m113 web-google:store-m105 roadnet-ca:store-m105; do
  g=${gs%%:*}
  ./dev_all.sh $g $D/${gs##*:}/$g
done
for g in lj orkut; do
  TAG=-up EXTRA="--up on" ./dev_all.sh $g $D/store-m113/$g rec
  ./dev_stab.sh $g $D/store-m113/$g capin sat capafter rec nest
  TAG=-up EXTRA="--up on" ./dev_stab.sh $g $D/store-m113/$g rec
done
./dev_determinism.sh > results/determinism.jsonl
for gm in lj:rec lj:capin lj:sat lj:capafter lj:nest orkut:rec orkut:capin web-google:rec web-google:capin roadnet-ca:rec roadnet-ca:capin; do
  LIM=8G ./map.sh ${gm%%:*} ${gm##*:} $D/csr/${gm%%:*} 8791 > /dev/null
done
python3 summarize.py results/dev/*.json > results/summary_small.txt
