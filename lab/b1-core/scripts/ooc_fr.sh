#!/bin/bash
# Larger-than-RAM: PageRank / WCC on Friendster under hard cgroup limits (targets.u32 = 14.4 GB).
# Uses a private copy of the CSR so that evicting its page cache does not hurt other experiments.
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; D=${D:-tmp/fr-a}; J=${J:-results/ooc_friendster.jsonl}; BL=/home/tux/graphlab/bench.lock; ML=/home/tux/graphlab/bigmem.lock
cg() { # cg LIMIT label cmd...
  local lim=$1 label=$2; shift 2
  $G evict --graph $D
  systemd-run --user --scope -q -p MemoryMax=$lim -p MemorySwapMax=0 "$@" --label "$label" --json $J
  local rc=$?
  echo "{\"cmd\":\"ooc-exit\",\"limit\":\"$lim\",\"label\":\"$label\",\"exit_code\":$rc}" | tee -a $J
}
part() { case $1 in
 base) # no limit: semi-external with O_DIRECT (cold by construction) and buffered
    $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 20 --label nolimit --json $J
    $G semiext --graph $D --algo wcc --io direct --threads 32 --label nolimit --json $J
    $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 5 --chunk-mb 2 --label nolimit-chunk2 --json $J
    $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 5 --chunk-mb 32 --label nolimit-chunk32 --json $J ;;
 16) cg 16G lim16-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 20
     cg 16G lim16-mmap-naive $G run --graph $D --algo pr32 --load mmap --threads 32 --iters 5
     cg 16G lim16-mmap-hint $G semiext --graph $D --algo pr32 --io mmap-hint --threads 32 --iters 5
     cg 16G lim16-wcc-semiext $G semiext --graph $D --algo wcc --io direct --threads 32
     cg 16G lim16-wcc-mmap-afforest $G run --graph $D --algo wcc --load mmap --threads 32 ;;
 8)  cg 8G lim8-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 20
     cg 8G lim8-semiext-direct-pr64 $G semiext --graph $D --algo pr64 --io direct --threads 32 --iters 5
     cg 8G lim8-wcc-semiext $G semiext --graph $D --algo wcc --io direct --threads 32
     cg 8G lim8-semiext-buffered $G semiext --graph $D --algo pr32 --io buffered --threads 32 --iters 3 ;;
 8m) cg 8G lim8-mmap-naive $G run --graph $D --algo pr32 --load mmap --threads 32 --iters 3
     cg 8G lim8-mmap-hint $G semiext --graph $D --algo pr32 --io mmap-hint --threads 32 --iters 3
     cg 8G lim8-wcc-mmap-afforest $G run --graph $D --algo wcc --load mmap --threads 32 ;;
 2)  cg 2G lim2-semiext-direct $G semiext --graph $D --algo pr32 --io direct --threads 32 --iters 5 --chunk-mb 4
     cg 1500M lim1.5-wcc-semiext $G semiext --graph $D --algo wcc --io direct --threads 32 --chunk-mb 4 ;;
esac; }
if [ -n "$1" ]; then part $1; else for p in base 16 8 8m 2; do flock $ML flock $BL $0 $p; done; fi
