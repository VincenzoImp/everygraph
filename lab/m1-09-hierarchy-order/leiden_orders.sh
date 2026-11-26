#!/usr/bin/env bash
# #146's fifth arm, in RAM only (m1-14, challenge 5): Leiden on the graph stored in a
# given order, visiting in storage (id) order or in ADR 0015's block-permuted order.
#   ./leiden_orders.sh <graph> "<orders>"      (orders: identity random bp hier-cap64 ...)
# Writes the permuted CSR to $S/csr/<graph>-<order>/ and appends one line per run to
# results/leiden_orders.jsonl. Exploratory: the machine is shared.
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-09-hierarchy-order}
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
B=${M109:-$S/target/release/m109}
B2=${B2:-$S/b2-target/release/b2}
g=$1; orders=$2
lim() { systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 "$@"; }
mkdir -p "$S/csr" "$S/leiden-orders" results
for o in $orders; do
  dir=$D/csr/$g
  if [ "$o" != identity ]; then
    dir=$S/csr/$g-$o
    [ -e "$dir/meta.json" ] || lim "$B" permute --graph "$D/csr/$g" --perm "$S/perm/$g/$o.perm" --out "$dir" 2>/dev/null
  fi
  for mode in async det; do
    for visit in block natural; do
      for rep in 1 2 3; do
        f=$S/leiden-orders/$g-$o-$mode-$visit-$rep.json
        flags=""; [ $mode = det ] && flags="--det"; [ $visit = natural ] && flags="$flags --natural"
        lim "$B2" run --graph "$dir" --name "$g" --algo leiden --threads 16 $flags --json "$f" > /dev/null 2>&1
        python3 -c "
import json; r = json.load(open('$f'))
print(json.dumps({'graph': '$g', 'order': '$o', 'mode': '$mode', 'visit': '$visit', 'rep': $rep,
  'time_s': r['time_s'], 'modularity': r['modularity'], 'communities': r['communities'], 'passes': r['n_passes']}))" >> results/leiden_orders.jsonl
      done
    done
  done
done
