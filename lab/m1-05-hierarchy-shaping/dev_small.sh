#!/usr/bin/env bash
# Every exploratory record of the small graphs, from scratch (shared machine: quality numbers
# only; they are deterministic). Writes results/dev/*.json, results/dev/*.stab.jsonl,
# results/capfix_lj.jsonl, results/determinism.jsonl, results/independence.jsonl,
# results/summary.txt. About an hour. Screenshots: map.sh and curate.sh (separately).
set -eu
cd "$(dirname "$0")"
D=/home/tux/graphlab/data
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
PY=${PY:-$S/venv/bin/python}
./dev_capfix.sh > results/capfix_lj.jsonl
for gs in lj:store-m113 orkut:store-m113 web-google:store-m105 roadnet-ca:store-m105; do
  g=${gs%%:*}
  ./dev_all.sh $g $D/${gs##*:}/$g
done
python3 summarize.py results/dev/*.json > results/summary.txt
: > results/independence.jsonl
for gs in web-google:store-m105 lj:store-m113; do
  g=${gs%%:*}
  ./lim.sh 8G $PY -I dev_independence.py $D/${gs##*:}/$g $S/$g $g >> results/independence.jsonl
done
./dev_determinism.sh > results/determinism.jsonl
for g in lj orkut; do
  ./dev_stab.sh $g $D/store-m113/$g > results/dev/$g.stab.txt
done
