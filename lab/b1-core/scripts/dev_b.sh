#!/bin/bash
# Fallback measurements pinned to the experiment's 6 dev CPUs (3 cores + SMT siblings). Not "official".
# Each step waits while one of my own official holds (scripts/h*.sh) is actively running.
cd /home/tux/graphlab/exp/b1-core
G=bin/glab-dev; TS="taskset -c 3-5,19-21"; T=6; D=tmp/fr-a; O=/home/tux/graphlab/data/csr/orkut
wait_idle() { while :; do busy=0; for l in logs/h?.log; do [ -s $l ] && ! grep -q "_DONE" $l && busy=1; done; [ $busy = 0 ] && break; sleep 3; done; }
export J=results/dev_ooc_friendster.jsonl
run() { wait_idle; "$@"; }
# in-RAM reference at the same thread count (page cache warm after the first run)
run $TS $G run --graph $D --algo wcc --threads $T --load mmap --reps 3 --label dev6-mmap-warm --json $J
run $TS $G run --graph $D --algo pr32 --threads $T --load mmap --iters 5 --label dev6-mmap-warm --json $J
run $TS $G semiext --graph $D --algo pr32 --io direct --threads $T --iters 5 --label dev6-nolimit-semiext-direct --json $J
run $TS $G semiext --graph $D --algo wcc --io direct --threads $T --label dev6-nolimit-wcc-semiext --json $J
run $TS scripts/cg.sh 8G 300 dev6-lim8-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads $T --iters 5
run $TS scripts/cg.sh 8G 120 dev6-lim8-wcc-semiext $G semiext --graph $D --algo wcc --io direct --threads $T
run $TS scripts/cg.sh 8G 300 dev6-lim8-mmap-naive $G run --graph $D --algo pr32 --load mmap --threads $T --iters 2
run $TS scripts/cg.sh 8G 300 dev6-lim8-mmap-hint $G semiext --graph $D --algo pr32 --io mmap-hint --threads $T --iters 2
run $TS scripts/cg.sh 8G 200 dev6-lim8-wcc-mmap-afforest $G run --graph $D --algo wcc --load mmap --threads $T
run $TS scripts/cg.sh 2G 200 dev6-lim2-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads $T --iters 3 --chunk-mb 4
echo DEV_B_DONE
J=results/dev_kernels_friendster.jsonl
run $TS $G run --graph $D --algo bfs --threads $T --load mmap --reps 2 --label dev6 --json $J
run $TS timeout 300 $G run --graph $D --algo kcore --threads $T --load mmap --label dev6 --json $J
echo DEV_A_DONE
