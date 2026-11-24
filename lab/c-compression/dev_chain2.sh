#!/bin/bash
# re-runs: BV cold (eviction fix) and twitter-sym svb/bp cold (first attempt coincided with IO PSI ~80% from other experiments)
cd "$(dirname "$0")"
export NOLOCK=1 THREADS=6 PIN="taskset -c 9-11,25-27"; D=/home/tux/graphlab/data/csr; R=results
./run_dev.sh orkut $D/orkut enc/orkut $R/dev6_cold.jsonl coldonly 1000 bv7 bv0
./run_dev.sh twsym $D/twitter-sym enc/twsym $R/dev6_cold.jsonl coldonly 1000 svb bp bv7 bv0
PREFIX="systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0" ./run_dev.sh twsym8g $D/twitter-sym enc/twsym $R/dev6_cold8g.jsonl coldonly 1000 bv7 bv0 svb
echo DEV2 DONE
