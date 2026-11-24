#!/usr/bin/env bash
# Friendster (65.6M nodes, 3.61B entries). Usage: ./run_friendster.sh <MemoryMax e.g. 34G|16G> <algo> <tag> [extra b2 args]
set -euo pipefail
cd "$(dirname "$0")"
MEM=$1; ALGO=$2; TAG=$3; shift 3
G=${GRAPH:-friendster}
mkdir -p results/big
flock /home/tux/graphlab/bigmem.lock flock /home/tux/graphlab/bench.lock \
  systemd-run --user --scope -q -p MemoryMax=$MEM -p MemorySwapMax=0 timeout 540 \
  ./target-w32/release/b2 run --graph /home/tux/graphlab/data/csr/$G --algo $ALGO --threads 32 \
     --lowmem --no-populate --verbose --json results/big/${G}_${ALGO}_${TAG}.json "$@" > logs/${G}_${ALGO}_${TAG}.log 2>&1
