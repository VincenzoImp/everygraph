#!/bin/bash
# HOLD 5 (bench lock): semi-external without limit and under 8 GB / 2 GB hard limits
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; D=tmp/fr-a; J=results/ooc_friendster.jsonl
$G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 10 --label nolimit-semiext-direct --json $J
$G semiext --graph $D --algo wcc --io direct --threads 32 --label nolimit-wcc-semiext --json $J
scripts/cg.sh 8G 200 lim8-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 20
scripts/cg.sh 8G 60 lim8-wcc-semiext $G semiext --graph $D --algo wcc --io direct --threads 32
scripts/cg.sh 8G 100 lim8-semiext-direct-pr64 $G semiext --graph $D --algo pr64 --io direct --threads 32 --iters 5
scripts/cg.sh 8G 150 lim8-mmap-naive $G run --graph $D --algo pr32 --load mmap --threads 32 --iters 3
scripts/cg.sh 8G 120 lim8-mmap-hint $G semiext --graph $D --algo pr32 --io mmap-hint --threads 32 --iters 3
scripts/cg.sh 8G 90 lim8-wcc-mmap-afforest $G run --graph $D --algo wcc --load mmap --threads 32
scripts/cg.sh 8G 80 lim8-semiext-buffered $G semiext --graph $D --algo pr32 --io buffered --threads 32 --iters 3
scripts/cg.sh 2G 60 lim2-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 5 --chunk-mb 4
scripts/cg.sh 1G 60 lim1-wcc-semiext $G semiext --graph $D --algo wcc --io direct --threads 32 --chunk-mb 2
echo H5_DONE
