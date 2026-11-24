#!/bin/bash
# friendster point queries (private copy) + constrained-RAM (graph >> RAM) steady state on friendster and twitter
cd "$(dirname "$0")"
sleep 2
while pgrep -f "run_pq3.sh|run_twitter.sh" >/dev/null; do sleep 5; done
G=work/csr/friendster
flock /home/tux/graphlab/bigmem.lock flock /home/tux/graphlab/bench.lock ./target/release/pq --graph $G --name friendster --threads 32 --cold 0 \
   --out results/pq_friendster_warm.json > /dev/null 2> logs/pq_friendster_warm.err
flock /home/tux/graphlab/bench.lock ./target/release/pq --graph $G --name friendster --threads 32 --warm 0 --prefetch 1 --random 1 --scale 0.5 \
   --out results/pq_friendster_cold_pf_rand.json > /dev/null 2> logs/pq_friendster_cold_pf_rand.err
flock /home/tux/graphlab/bench.lock ./target/release/pq --graph $G --name friendster --threads 32 --warm 0 --prefetch 1 --random 0 --scale 0.5 \
   --out results/pq_friendster_cold_pf_norm.json > /dev/null 2> logs/pq_friendster_cold_pf_norm.err
# graph much larger than RAM: cgroup caps page cache + heap at 3 GiB; no pre-warm; steady state after the first queries
flock /home/tux/graphlab/bench.lock systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 ./target/release/pq --graph $G --name friendster_ram3g \
   --threads 32 --cold 0 --prewarm 0 --evict_first 1 --prefetch 1 --random 1 --scale 0.5 \
   --out results/pq_friendster_ram3g.json > /dev/null 2> logs/pq_friendster_ram3g.err
flock /home/tux/graphlab/bench.lock systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 ./target/release/pq --graph work/csr/twitter-dir \
   --in work/csr/twitter-dir-in --name twitter-dir_ram3g --threads 32 --cold 0 --prewarm 0 --evict_first 1 --prefetch 1 --random 1 --scale 0.5 \
   --out results/pq_twitter-dir_ram3g.json > /dev/null 2> logs/pq_twitter_ram3g.err
