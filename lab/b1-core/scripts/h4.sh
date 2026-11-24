#!/bin/bash
# HOLD 4 (bigmem + bench locks): 32 GB-budget Friendster ingest, heap PageRank, 16 GB cgroup limit runs
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; D=tmp/fr-a; P=/home/tux/graphlab/data/csr/friendster
$G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --mem-gb 32 --threads 32 --direct-in --out-mmap --label fr-budget32-directin-outmmap --json results/ingest.jsonl
for f in offsets.u64 targets.u32; do cmp tmp/fr-b/$f $P/$f && echo "VALID(budget32) tmp/fr-b $f identical to published"; done
$G run --graph $D --algo pr32 --threads 32 --load heap --iters 10 --label heap --json results/kernels_friendster.jsonl
scripts/cg.sh 16G 200 lim16-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 5
scripts/cg.sh 16G 150 lim16-mmap-naive $G run --graph $D --algo pr32 --load mmap --threads 32 --iters 5
scripts/cg.sh 16G 90 lim16-wcc-mmap-afforest $G run --graph $D --algo wcc --load mmap --threads 32
echo H4_DONE
