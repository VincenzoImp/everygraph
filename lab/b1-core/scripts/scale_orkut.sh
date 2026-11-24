#!/bin/bash
# thread scaling of in-RAM kernels on orkut (call under the bench lock)
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; D=/home/tux/graphlab/data/csr/orkut; J=${J:-results/kernels_orkut.jsonl}
for T in ${THREADS:-32 16 8 4 2 1}; do
  for algo in ${ALGOS:-pr64 pr32 wcc wcc-stream bfs kcore tc}; do
    reps=3; case $algo in pr64|pr32|tc) reps=1;; esac
    [ $T -ge 16 ] && reps=3
    $G run --graph $D --algo $algo --threads $T --load heap --reps $reps --label scale --json $J
  done
done
