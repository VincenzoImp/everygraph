#!/bin/bash
# Friendster in-RAM kernels (mmap-warm unless stated). Needs bigmem + bench locks; each lock hold < 10 min.
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; D=${D:-tmp/fr-a}; J=${J:-results/kernels_friendster.jsonl}; BL=/home/tux/graphlab/bench.lock; ML=/home/tux/graphlab/bigmem.lock
L="flock $ML flock $BL"
part() { case $1 in
 a) $G run --graph $D --algo wcc --threads 32 --load mmap --reps 2 --label warmup --json $J   # warms the page cache
    for T in 32 16 8 4; do $G run --graph $D --algo wcc --threads $T --load mmap --reps 2 --label scale --json $J; $G run --graph $D --algo bfs --threads $T --load mmap --reps 2 --label scale --json $J; done
    $G run --graph $D --algo pr32 --threads 32 --load mmap --iters 20 --label scale --json $J
    $G run --graph $D --algo pr64 --threads 32 --load mmap --iters 20 --label scale --json $J
    $G run --graph $D --algo pr32 --threads 16 --load mmap --iters 20 --label scale --json $J ;;
 b) $G run --graph $D --algo pr32 --threads 8 --load mmap --iters 5 --label scale --json $J
    $G run --graph $D --algo pr32 --threads 4 --load mmap --iters 5 --label scale --json $J
    $G run --graph $D --algo pr32 --threads 2 --load mmap --iters 2 --label scale --json $J
    $G run --graph $D --algo pr32 --threads 1 --load mmap --iters 2 --label scale --json $J
    for T in 2 1; do $G run --graph $D --algo wcc --threads $T --load mmap --label scale --json $J; $G run --graph $D --algo bfs --threads $T --load mmap --label scale --json $J; done ;;
 c) $G run --graph $D --algo pr32 --threads 32 --load heap --iters 20 --label heap --json $J
    $G run --graph $D --algo wcc-stream --threads 32 --load mmap --label scale --json $J
    $G run --graph $D --algo kcore --threads 32 --load mmap --label scale --json $J
    $G run --graph $D --algo kcore --threads 16 --load mmap --label scale --json $J
    $G run --graph $D --algo tc --threads 32 --load mmap --label scale --json $J ;;
 d) $G run --graph $D --algo tc --threads 16 --load mmap --label scale --json $J
    $G run --graph $D --algo kcore --threads 8 --load mmap --label scale --json $J ;;
esac; }
if [ -n "$1" ]; then part $1; else for p in a b c d; do $L $0 $p; done; fi
