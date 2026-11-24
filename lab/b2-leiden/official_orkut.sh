#!/usr/bin/env bash
# orkut official scaling, single repetition, most informative thread counts first (one bench.lock acquisition).
cd "$(dirname "$0")"
export B=./target/release/b2 DATA=/home/tux/graphlab/data/csr g=${G:-orkut}
mkdir -p results/scaling
flock /home/tux/graphlab/bench.lock bash -c '
  for t in 32 16 8 4 2 1; do for algo in leiden louvain; do for mode in async det; do
    out=results/scaling/${g}_${algo}_${mode}_t$t.json; [ -e $out ] && continue
    extra=""; [ "$mode" = det ] && extra="--det"
    timeout 200 $B run --graph $DATA/$g --algo $algo --threads $t $extra --json $out > /dev/null
  done; done; done'
