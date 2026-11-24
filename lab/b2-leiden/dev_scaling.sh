#!/usr/bin/env bash
# Fallback thread scaling restricted to the assigned dev cores (3 physical cores 6-8 + SMT siblings 22-24), no lock.
cd "$(dirname "$0")"
B=./target/release/b2; DATA=/home/tux/graphlab/data/csr
mkdir -p results/dev_scaling
for g in ${GRAPHS:-lj orkut}; do for algo in louvain leiden; do for mode in async det; do
  extra=""; [ "$mode" = det ] && extra="--det"
  for t in 1 2 3 6; do
    cpus="6,7,8"; [ $t = 6 ] && cpus="6-8,22-24"; [ $t = 1 ] && cpus="6"; [ $t = 2 ] && cpus="6,7"
    rep=2; [ $t = 1 ] && rep=1
    taskset -c $cpus $B run --graph $DATA/$g --algo $algo --threads $t --repeat $rep $extra --json results/dev_scaling/${g}_${algo}_${mode}_t$t.json > /dev/null
  done
done; done; done
