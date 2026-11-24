#!/bin/bash
# Fallback measurements on the assigned dev cores (6 threads) because bench.lock was heavily contended.
cd "$(dirname "$0")"
export NOLOCK=1 THREADS=6 PIN="taskset -c 9-11,25-27"; D=/home/tux/graphlab/data/csr; R=results
for f in enc/orkut/raw.bin enc/twsym/svb.bin; do ./glc readbw $f 6 >> $R/readbw.jsonl; ./glc readbw $f 32 >> $R/readbw.jsonl; done
./run_dev.sh orkut $D/orkut enc/orkut $R/dev6_cold.jsonl coldonly 1000 raw varint svb bp zstd3 lz4 vzstd3 bv7 bv0
./run_dev.sh twsym $D/twitter-sym enc/twsym $R/dev6_cold.jsonl coldonly 1000 varint svb bp zstd3 lz4 vzstd3 bv7 bv0
PREFIX="systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0" ./run_dev.sh twsym8g $D/twitter-sym enc/twsym $R/dev6_cold8g.jsonl coldonly 1000 raw varint svb bp vzstd3 bv7 bv0
./run_order.sh $D/orkut enc/orkut ord/orkut 6 $R/dev6_order.jsonl
./run_dev.sh twsym $D/twitter-sym enc/twsym $R/dev6_bakeoff.jsonl warm 100000 bv7
echo DEV DONE
