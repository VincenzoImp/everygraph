#!/bin/bash
# HOLD 3 (bench lock): Friendster kernels, CSR memory-mapped (private copy), page cache warm after the first run
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; D=${D:-tmp/fr-a}; J=${J:-results/kernels_friendster.jsonl}
$G run --graph $D --algo wcc --threads 32 --load mmap --reps 3 --label warmup+scale --json $J
$G run --graph $D --algo bfs --threads 32 --load mmap --reps 3 --label scale --json $J
$G run --graph $D --algo pr32 --threads 32 --load mmap --iters 20 --label scale --json $J
$G run --graph $D --algo pr64 --threads 32 --load mmap --iters 20 --label scale --json $J
for T in 16 8 4; do
  $G run --graph $D --algo wcc --threads $T --load mmap --reps 2 --label scale --json $J
  $G run --graph $D --algo bfs --threads $T --load mmap --reps 2 --label scale --json $J
done
$G run --graph $D --algo pr32 --threads 16 --load mmap --iters 10 --label scale --json $J
$G run --graph $D --algo pr32 --threads 8 --load mmap --iters 5 --label scale --json $J
$G run --graph $D --algo pr32 --threads 4 --load mmap --iters 3 --label scale --json $J
$G run --graph $D --algo wcc-stream --threads 32 --load mmap --label scale --json $J
timeout 120 $G run --graph $D --algo kcore --threads 32 --load mmap --label scale --json $J || echo '{"cmd":"timeout","what":"kcore friendster T32","limit_s":120}' | tee -a $J
timeout 150 $G run --graph $D --algo tc --threads 32 --load mmap --label scale --json $J || echo '{"cmd":"timeout","what":"tc friendster T32","limit_s":150}' | tee -a $J
echo H3_DONE
