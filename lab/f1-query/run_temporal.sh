#!/bin/bash
cd "$(dirname "$0")"
for g in orkut lj; do
taskset -c 31 ./target/release/props --graph work/csr/$g --work work/props_$g --threads 1 --reps 3 --only temporal \
  --out results/props_${g}_temporal_t1.json > /dev/null 2> logs/props_${g}_temporal_t1.err
done
