#!/bin/bash
# Development probe (not for the record): how long the big-graph runs take, to size bench.sh.
S=${SCRATCH:-/home/tux/graphlab/scratch/m1-13}; D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}/csr; B=$S/target/release/m113
date
$B kernel --graph $D/twitter-sym --store $S/store/twitter-sym --config raw --threads 16 --algos tc,tc-comp --reps 1 --json $S/probe.jsonl
date
$B leiden run --graph $D/twitter-sym --store $S/store/twitter-sym --config raw --mem heap --algo leiden --threads 16 --repeat 1 --json $S/probe.jsonl
date
$B kernel --graph $D/twitter-sym --store $S/store/twitter-sym --config raw --threads 1 --algos pr32,wcc,bfs,kcore --reps 1 --json $S/probe.jsonl
date
$B kernel --graph $D/friendster --store $S/store/friendster --config raw --threads 16 --algos pr32,wcc,bfs,kcore,tc --reps 1 --json $S/probe.jsonl
date
$B kernel --graph $D/friendster --store $S/store/friendster --config comp --threads 16 --algos pr32,wcc,bfs,kcore,tc --reps 1 --json $S/probe.jsonl
date
echo PROBE DONE
