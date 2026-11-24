#!/usr/bin/env bash
# Quality / hierarchy variants on the dev cores (6 hw threads = 3 cores + SMT), no lock. Timings are NOT headline numbers.
cd "$(dirname "$0")"
B=./target/release/b2; DATA=/home/tux/graphlab/data/csr; T="taskset -c 6-8,22-24"
mkdir -p results/dev_quality results/dev_hier
for g in ${GRAPHS:-lj orkut}; do
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --iterations 3 --json results/dev_quality/${g}_leiden_async_it3.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --iterations 3 --det --json results/dev_quality/${g}_leiden_det_it3.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 3 --iterations 3 --det --json results/dev_quality/${g}_leiden_det_it3_t3.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --iterations 5 --json results/dev_quality/${g}_leiden_async_it5.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --natural --json results/dev_quality/${g}_leiden_async_naturalorder.json > /dev/null
  $T $B run --graph $DATA/$g --algo louvain --threads 6 --natural --json results/dev_quality/${g}_louvain_async_naturalorder.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --json results/dev_hier/${g}_leiden_natural.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --min-ratio 3 --json results/dev_hier/${g}_leiden_natural_ratio3.json > /dev/null
  $T $B run --graph $DATA/$g --algo louvain --threads 6 --json results/dev_hier/${g}_louvain_natural.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --cap 64 --max-passes 20 --min-ratio 3 --json results/dev_hier/${g}_leiden_cap64_ratio3.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --cap 8 --max-passes 30 --min-ratio 6 --json results/dev_hier/${g}_leiden_cap8_ratio6.json > /dev/null
  $T $B run --graph $DATA/$g --algo leiden --threads 6 --det --cap 64 --max-passes 20 --min-ratio 3 --json results/dev_hier/${g}_leiden_det_cap64_ratio3.json > /dev/null
  $T $B run --graph $DATA/$g --algo louvain --threads 6 --cap 32 --max-passes 20 --min-ratio 3 --json results/dev_hier/${g}_louvain_cap32_ratio3.json > /dev/null
done
