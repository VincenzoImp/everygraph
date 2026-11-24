#!/bin/bash
# design probes on orkut, 32 threads (call under the bench lock)
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; D=/home/tux/graphlab/data/csr/orkut; J=${J:-results/probes_orkut.jsonl}; T=${T:-32}
for rep in 1; do
for algo in pr64 pr32; do
  # (a) mmap-warm vs heap, (d) huge pages
  for load in mmap heap heap-huge heap-nohuge; do
    $G run --graph $D --algo $algo --threads $T --load $load --reps 3 --label probe-load --json $J
  done
  $G run --graph $D --algo $algo --threads $T --load heap --thp off --reps 3 --label probe-thp-off --json $J
  # (b) u64 ids
  $G run --graph $D --algo $algo --threads $T --load heap --ids u64 --reps 3 --label probe-ids --json $J
  # (c) node order
  for order in degree bfs random; do
    $G run --graph $D --algo $algo --threads $T --load heap --order $order --reps 3 --label probe-order --json $J
  done
done
done
