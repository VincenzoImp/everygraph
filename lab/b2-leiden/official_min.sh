#!/usr/bin/env bash
# Minimal official run (one bench.lock acquisition per graph): thread scaling async vs det, Louvain + Leiden. Skips existing JSONs.
cd "$(dirname "$0")"
export B=./target/release/b2 DATA=/home/tux/graphlab/data/csr
mkdir -p results/scaling
for g in ${GRAPHS:-lj orkut}; do
  export g
  flock /home/tux/graphlab/bench.lock bash -c '
    for algo in louvain leiden; do for mode in async det; do for t in 32 16 8 4 2 1; do
      out=results/scaling/${g}_${algo}_${mode}_t$t.json; [ -e $out ] && continue
      extra=""; [ "$mode" = det ] && extra="--det"
      rep=2; [ "$t" -le 2 ] && rep=1
      timeout 300 $B run --graph $DATA/$g --algo $algo --threads $t --repeat $rep $extra --json $out > /dev/null
    done; done; done'
done
