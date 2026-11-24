#!/bin/bash
# dev-core runs of Q1 (4 threads); args: sizes...
cd "$(dirname "$0")"
for n in "$@"; do
 for kind in eth user; do
  for m in hash mphf fst sorted fc; do
   RAYON_NUM_THREADS=4 taskset -c 14-15,30-31 nice -n 5 ./target/release/dict --method $m --kind $kind --n $n --dir work --threads 4 \
     --out results/dict_${m}_${kind}_${n}_t4.json > /dev/null 2> logs/dict_${m}_${kind}_${n}.err
  done
 done
 RAYON_NUM_THREADS=4 taskset -c 14-15,30-31 nice -n 5 ./target/release/dict --method fst --kind ethbin --n $n --dir work --threads 4 \
     --out results/dict_fst_ethbin_${n}_t4.json > /dev/null 2> logs/dict_fst_ethbin_${n}.err
done
