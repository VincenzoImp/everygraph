#!/bin/bash
# point queries on twitter-2010 directed (out = twitter-dir, in = twitter-dir-in produced by pq --transpose).
# Uses private copies (eviction must work); ~12.4 GB resident -> bigmem + bench locks.
cd "$(dirname "$0")"
D=/home/tux/graphlab/data/csr
for g in twitter-dir twitter-dir-in; do
  [ -f work/csr/$g/targets.u32 ] || { mkdir -p work/csr/$g; cp $D/$g/{offsets.u64,targets.u32,meta.json} work/csr/$g/; }
done
for mode in "warm:--cold 0" "cold_pf_rand:--warm 0 --prefetch 1 --random 1 --scale 0.5" "cold_pf_norm:--warm 0 --prefetch 1 --random 0 --scale 0.5"; do
  tag=${mode%%:*}; a=${mode#*:}
  flock /home/tux/graphlab/bigmem.lock flock /home/tux/graphlab/bench.lock ./target/release/pq --graph work/csr/twitter-dir --in work/csr/twitter-dir-in --name twitter-dir \
     --threads 32 $a --out results/pq_twitter-dir_${tag}.json > /dev/null 2> logs/pq_twitter_${tag}.err
done
