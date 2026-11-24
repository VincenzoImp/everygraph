#!/bin/bash
# The official (lock-holding) measurement chain used for REPORT.md (bench.sh contains the same steps).
cd "$(dirname "$0")"
DATA=/home/tux/graphlab/data/csr; R=results
./run_bakeoff.sh orkut $DATA/orkut enc/orkut $R/bakeoff.jsonl warm 200000 raw varint svb bp zstd3 lz4 bv7 bv0 bvd7
./run_bakeoff.sh uk $PWD/g/uk-csr enc/uk $R/bakeoff.jsonl warm 200000 raw varint svb bp zstd3 lz4 bv7 bv0
BIG=1 ./run_bakeoff.sh twsym $DATA/twitter-sym enc/twsym $R/bakeoff.jsonl warm 200000 raw varint svb bp bv7 bv0
BIG=1 ./run_bakeoff.sh twdir $DATA/twitter-dir g/twitter $R/bakeoff.jsonl warm 200000 twbv
for f in enc/orkut/raw.bin enc/twsym/raw.bin; do flock /home/tux/graphlab/bench.lock ./glc readbw $f 32 >> $R/readbw.jsonl; done
./run_bakeoff.sh orkut $DATA/orkut enc/orkut $R/cold.jsonl coldonly 1000 raw varint svb bp zstd3 lz4 bv7 bv0
BIG=1 ./run_bakeoff.sh twsym $DATA/twitter-sym enc/twsym $R/cold.jsonl coldonly 1000 raw varint svb bp zstd3 lz4 bv7 bv0
PREFIX="systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0" BIG=1 \
  ./run_bakeoff.sh twsym8g $DATA/twitter-sym enc/twsym $R/cold8g.jsonl coldonly 1000 raw varint svb bp bv7 bv0
echo OFFICIAL DONE
