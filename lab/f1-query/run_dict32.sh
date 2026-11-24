#!/bin/bash
# 32-thread dictionary runs (bench lock), plus a real 500M-key MPHF build (bigmem lock, binary 20-byte keys)
cd "$(dirname "$0")"
L="flock /home/tux/graphlab/bench.lock"
for m in hash mphf fc; do
  $L env RAYON_NUM_THREADS=32 ./target/release/dict --method $m --kind eth --n 65000000 --dir work --threads 32 \
     --out results/dict_${m}_eth_65000000_t32.json > /dev/null 2> logs/dict32_${m}.err
done
$L env RAYON_NUM_THREADS=32 ./target/release/dict --method dedupe --kind eth --n 65000000 --dir work --threads 32 \
   --out results/dict_dedupe_eth_65000000_t32.json > /dev/null 2> logs/dict32_dedupe.err
flock /home/tux/graphlab/bigmem.lock $L systemd-run --user --scope -q -p MemoryMax=33G -p MemorySwapMax=0 env RAYON_NUM_THREADS=32 \
   ./target/release/dict --method mphf --kind ethbin --n 500000000 --dir work --threads 32 --queries 20000000 \
   --out results/dict_mphf_ethbin_500000000_t32.json > /dev/null 2> logs/dict32_mphf_500M.err
