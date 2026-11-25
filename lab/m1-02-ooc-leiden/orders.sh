#!/usr/bin/env bash
# Survey ideas L1-L3 (docs/research/memory-bounded-algorithms.md § 2), exploratory: visiting
# orders on lj and orkut, async Leiden, 1 iteration, 16 threads, compressed store.
#   in RAM (warm, 8 GiB limit): Q, communities, time per order, 3 runs each
#   out of core (cold, LIMIT_<graph>): time, bytes read from the disk, Q, with the L3 trace
# Orders: block   = ADR 0015 (blocks of 64 ids, permuted over the whole graph)
#         natural = id order
#         winW    = blocks permuted inside fixed windows of W blocks, windows in disk order (L2)
#         shuf-*  = the same on a seeded random relabelling of the ids (L1)
# Usage: ./orders.sh  (needs the stores; see PREP below). Rows go to $OUT (default results/orders.jsonl).
set -u
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-02-ooc-leiden}
D=/home/tux/graphlab/data
B=./target/release/m102
export T=16
mkdir -p "$S/csr"
# PREP: shuffled CSR and its store (seed 1)
for g in lj orkut; do
  if [ ! -e "$D/store-m113/$g-shuf1/meta.json" ]; then
    systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 $B shuffle --graph $D/csr/$g --out $S/csr/$g-shuf1 --seed 1
    systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 $B encode --graph $S/csr/$g-shuf1 --out $D/store-m113/$g-shuf1 --threads 16 >> "$S/orders_encode.jsonl"
  fi
done
store() { echo "$D/store-m113/$1"; }
flags() {
  case $1 in
    block) echo "" ;;
    natural) echo "--natural" ;;
    win*) echo "--order-window ${1#win}" ;;
  esac
}
ORDERS=${ORDERS:-"block natural win16 win256 win4096 win65536"}
export OUT="${OUT:-$PWD/results/orders.jsonl}"
for g in lj orkut lj-shuf1 orkut-shuf1; do
  for o in $ORDERS; do
    case $g in *-shuf1) [ "$o" = block ] || [ "$o" = natural ] || continue ;; esac
    for rep in 1 2 3; do
      echo "== inram $g $o rep $rep"
      WARM=1 ./dev.sh 8G --store "$(store $g)" --name "$g" --mem mmap $(flags $o) --notes "inram $o"
    done
  done
done
LIMIT_lj=${LIMIT_lj:-300M}
LIMIT_orkut=${LIMIT_orkut:-447M}
for g in lj orkut lj-shuf1 orkut-shuf1; do
  base=${g%-shuf1}
  lim_var=LIMIT_$base
  for o in $ORDERS; do
    case $g in *-shuf1) [ "$o" = block ] || [ "$o" = natural ] || continue ;; esac
    echo "== ooc $g $o ${!lim_var}"
    ./dev.sh "${!lim_var}" --store "$(store $g)" --name "$g" --mem mmap-lazy --ooc --spill "$S/spill" --trace-io $(flags $o) --notes "ooc $o"
  done
done
