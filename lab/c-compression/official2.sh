#!/bin/bash
cd "$(dirname "$0")"
DATA=/home/tux/graphlab/data/csr; R=results
./run_bakeoff.sh orkut $DATA/orkut enc/orkut $R/bakeoff.jsonl warm 200000 vzstd3
./run_bakeoff.sh uk $PWD/g/uk-csr enc/uk $R/bakeoff.jsonl warm 200000 vzstd3
BIG=1 ./run_bakeoff.sh twsym $DATA/twitter-sym enc/twsym $R/bakeoff.jsonl warm 200000 vzstd3
./run_bakeoff.sh orkut $DATA/orkut enc/orkut $R/cold.jsonl coldonly 1000 vzstd3
BIG=1 ./run_bakeoff.sh twsym $DATA/twitter-sym enc/twsym $R/cold.jsonl coldonly 1000 vzstd3
PREFIX="systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0" BIG=1 ./run_bakeoff.sh twsym8g $DATA/twitter-sym enc/twsym $R/cold8g.jsonl coldonly 1000 vzstd3
flock /home/tux/graphlab/bench.lock ./run_order.sh $DATA/orkut enc/orkut ord/orkut 32 $R/order.jsonl
echo OFFICIAL2 DONE
