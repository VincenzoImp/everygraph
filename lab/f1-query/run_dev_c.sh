#!/bin/bash
# remaining point-query runs on the dev cores (bench lock queue was ~10 deep); throughput with 4 threads here
cd "$(dirname "$0")"
sleep 1
while pgrep -f "release/props --graph work/csr/orkut" >/dev/null; do sleep 5; done
TS="taskset -c 14-15,30-31"
TW="--graph work/csr/twitter-dir --in work/csr/twitter-dir-in"
FR="--graph work/csr/friendster"
$TS ./target/release/pq $TW --name twitter-dir --threads 4 --warm 0 --prefetch 1 --random 1 --scale 0.5 --out results/pq_twitter-dir_cold_pf_rand.json >/dev/null 2>logs/pq_twitter_cold_pf_rand.err
$TS ./target/release/pq $TW --name twitter-dir --threads 4 --warm 0 --prefetch 1 --random 0 --scale 0.5 --out results/pq_twitter-dir_cold_pf_norm.json >/dev/null 2>logs/pq_twitter_cold_pf_norm.err
$TS ./target/release/pq $FR --name friendster --threads 4 --warm 0 --prefetch 1 --random 1 --scale 0.5 --out results/pq_friendster_cold_pf_rand.json >/dev/null 2>logs/pq_friendster_cold_pf_rand.err
$TS ./target/release/pq $FR --name friendster --threads 4 --warm 0 --prefetch 1 --random 0 --scale 0.5 --out results/pq_friendster_cold_pf_norm.json >/dev/null 2>logs/pq_friendster_cold_pf_norm.err
systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 $TS ./target/release/pq $FR --name friendster_ram3g --threads 4 --cold 0 --prewarm 0 --evict_first 1 \
   --prefetch 1 --random 1 --scale 0.5 --out results/pq_friendster_ram3g.json >/dev/null 2>logs/pq_friendster_ram3g.err
systemd-run --user --scope -q -p MemoryMax=3G -p MemorySwapMax=0 $TS ./target/release/pq $TW --name twitter-dir_ram3g --threads 4 --cold 0 --prewarm 0 --evict_first 1 \
   --prefetch 1 --random 1 --scale 0.5 --out results/pq_twitter-dir_ram3g.json >/dev/null 2>logs/pq_twitter_ram3g.err
for g in orkut lj; do
  $TS ./target/release/pq --graph work/csr/$g --name $g --threads 4 --cold 0 --out results/pq_${g}_warm_t4.json >/dev/null 2>logs/pq_${g}_warm_t4.err
done
$TS ./target/release/pq $TW --verify 1 --verify_n 10 > logs/verify_twitter.out 2>&1
