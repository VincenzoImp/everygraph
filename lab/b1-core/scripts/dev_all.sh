#!/bin/bash
# Fallback measurements pinned to the experiment's 6 dev CPUs (3 cores + SMT siblings). Not "official".
cd /home/tux/graphlab/exp/b1-core
G=bin/glab-dev; TS="taskset -c 3-5,19-21"; T=6; D=tmp/fr-a; O=/home/tux/graphlab/data/csr/orkut
# (c) friendster ingest, 8 GB budget, all-O_DIRECT temp I/O
$TS $G ingest --input tmp/friendster.txt --out tmp/fr-c --tmp tmp/t3 --mem-gb 8 --threads $T --direct-in --direct-tmp --out-mmap --label dev6-fr-budget8-directin-directtmp-outmmap --json results/dev_ingest.jsonl
for f in offsets.u64 targets.u32 ids.u64; do cmp tmp/fr-c/$f /home/tux/graphlab/data/csr/friendster/$f && echo "VALID tmp/fr-c $f identical to published"; done
echo DEV_C_DONE
# (b) out-of-core under an 8 GB hard limit
export J=results/dev_ooc_friendster.jsonl
$TS $G semiext --graph $D --algo pr32 --io direct --threads $T --iters 5 --label dev6-nolimit-semiext-direct --json $J
$TS scripts/cg.sh 8G 300 dev6-lim8-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads $T --iters 5
$TS scripts/cg.sh 8G 120 dev6-lim8-wcc-semiext $G semiext --graph $D --algo wcc --io direct --threads $T
$TS scripts/cg.sh 8G 300 dev6-lim8-mmap-naive $G run --graph $D --algo pr32 --load mmap --threads $T --iters 2
$TS scripts/cg.sh 8G 300 dev6-lim8-mmap-hint $G semiext --graph $D --algo pr32 --io mmap-hint --threads $T --iters 2
$TS scripts/cg.sh 8G 200 dev6-lim8-wcc-mmap-afforest $G run --graph $D --algo wcc --load mmap --threads $T
$TS scripts/cg.sh 2G 200 dev6-lim2-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads $T --iters 3 --chunk-mb 4
echo DEV_B_DONE
# (a) friendster kernels, mmap warm
J=results/dev_kernels_friendster.jsonl
$TS $G run --graph $D --algo wcc --threads $T --load mmap --reps 3 --label dev6 --json $J
$TS $G run --graph $D --algo bfs --threads $T --load mmap --reps 2 --label dev6 --json $J
$TS $G run --graph $D --algo pr32 --threads $T --load mmap --iters 5 --label dev6 --json $J
$TS $G run --graph $D --algo pr64 --threads $T --load mmap --iters 5 --label dev6 --json $J
$TS timeout 300 $G run --graph $D --algo kcore --threads $T --load mmap --label dev6 --json $J
echo DEV_A_DONE
# (d) probes on orkut
J=results/dev_probes_orkut.jsonl
for algo in pr32 pr64; do
  for load in mmap heap; do $TS $G run --graph $O --algo $algo --threads $T --load $load --reps 3 --label dev6-probe --json $J; done
  $TS $G run --graph $O --algo $algo --threads $T --load heap --thp off --reps 3 --label dev6-probe --json $J
  $TS $G run --graph $O --algo $algo --threads $T --load heap --ids u64 --reps 3 --label dev6-probe --json $J
  for order in degree bfs random; do $TS $G run --graph $O --algo $algo --threads $T --load heap --order $order --reps 3 --label dev6-probe --json $J; done
done
echo DEV_D_DONE
