#!/bin/bash
cd "$(dirname "$0")"
while pgrep -f run_twitter.sh > /dev/null; do sleep 5; done
./run_pq.sh orkut lj
./run_twitter.sh
