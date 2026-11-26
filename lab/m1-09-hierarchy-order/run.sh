#!/usr/bin/env bash
# The ordering bake-off on one graph (m1-09, #23). Exploratory on a shared machine: kernel
# times are interleaved A/B ratios in one process, never absolute numbers for a document.
#   ./run.sh <graph> <MemoryMax> "<orders>" [sources]
# orders: random bfs dbg flat hier hier-deg hier-bfs hier-cap64 bp bp-input bp-nocool bp-self bp-full llp identity
# sources: raw | compressed | raw,compressed (the kernels' adjacency; default raw)
# MMAP=1: the input targets memory-mapped instead of copied to the heap (twitter-sym, friendster)
# Needs: the Leiden hierarchy (./leiden.sh <graph> natural|cap64) for the hier* orders,
# and $S/perm/<graph>/llp.perm (./webgraph.sh llp <graph> <MemoryMax>) for llp.
# Appends to results/orders.jsonl (time to compute each order) and results/eval.jsonl.
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-09-hierarchy-order}
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
B=${M109:-$S/target/release/m109}
g=$1; lim=$2; orders=$3; sources=${4:-raw}
P=$S/perm/$g
mkdir -p "$P" results
lim() { systemd-run --user --scope -q -p MemoryMax="$lim" -p MemorySwapMax=0 "$@"; }
for o in $orders; do
  [ "$o" = identity ] && continue
  [ "$o" = llp ] && continue
  [ -e "$P/$o.perm" ] && continue
  case $o in
    hier-cap64) args="--method hier --hier $S/hier/$g-cap64" ;;
    flat|hier|hier-deg|hier-bfs) args="--method $o --hier $S/hier/$g-natural" ;;
    bp-input) args="--method bp --start input" ;;
    bp-nocool) args="--method bp --tol-step 0" ;;
    bp-self) args="--method bp --self-term 1" ;;
    bp-full) args="--method bp --depth 40" ;;
    *) args="--method $o" ;;
  esac
  lim "$B" order --graph "$D/csr/$g" $args --seed 1 --threads 16 --mmap "${MMAP:-0}" --out "$P/$o.perm" \
    | sed "s/^{/{\"name\": \"$o\", /" >> results/orders.jsonl
done
for o in $orders; do
  perm=$P/$o.perm; [ "$o" = identity ] && perm=identity
  lim "$B" eval --graph "$D/csr/$g" --perm "$perm" --name "$o" --threads 16 --mmap "${MMAP:-0}" --reps "${REPS:-3}" \
    --kernels "${KERNELS:-pr,bfs,wcc}" --sources "$sources" >> results/eval.jsonl
done
