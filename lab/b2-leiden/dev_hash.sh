#!/usr/bin/env bash
# Determinism check at high thread counts WITHOUT using the whole machine: 4/16/32 worker threads confined to the
# 6 dev hw threads (heavily oversubscribed = adversarial scheduling). Only hashes matter, timings are meaningless.
cd "$(dirname "$0")"
B=./target/release/b2; DATA=/home/tux/graphlab/data/csr
for g in lj orkut; do for algo in louvain leiden; do for t in 1 4 16 32; do
  out=results/dev_hash/${g}_${algo}_det_t$t.json; [ -e $out ] && continue
  taskset -c 6-8,22-24 $B run --graph $DATA/$g --algo $algo --threads $t --det --json $out > /dev/null
done; done; done
