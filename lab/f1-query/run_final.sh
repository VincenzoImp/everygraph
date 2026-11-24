#!/bin/bash
# rerun twitter cold modes (first attempt overlapped with a process that kept the files mapped), then verify directed bibfs
cd "$(dirname "$0")"
sleep 2
while pgrep -f "run_big.sh|run_twitter.sh" >/dev/null; do sleep 5; done
for mode in "cold_pf_rand:--warm 0 --prefetch 1 --random 1 --scale 0.5" "cold_pf_norm:--warm 0 --prefetch 1 --random 0 --scale 0.5"; do
  tag=${mode%%:*}; a=${mode#*:}
  flock /home/tux/graphlab/bench.lock ./target/release/pq --graph work/csr/twitter-dir --in work/csr/twitter-dir-in --name twitter-dir \
     --threads 32 $a --out results/pq_twitter-dir_${tag}.json > /dev/null 2> logs/pq_twitter_${tag}.err
done
taskset -c 14-15,30-31 ./target/release/pq --graph work/csr/twitter-dir --in work/csr/twitter-dir-in --verify 1 --verify_n 15 > logs/verify_twitter.out 2>&1
