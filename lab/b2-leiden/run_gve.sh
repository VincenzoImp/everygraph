#!/usr/bin/env bash
# Runs the GVE-Louvain / GVE-Leiden binaries built by exp/a-baselines (read-only use) on the same graphs, same machine state.
cd "$(dirname "$0")"
A=/home/tux/graphlab/exp/a-baselines
for g in lj orkut; do for a in louvain leiden; do
  [ -x $A/gve/gve-$a-t32 ] && [ -e $A/data/$g.mtx ] || continue
  flock /home/tux/graphlab/bench.lock $A/gve/gve-$a-t32 $A/data/$g.mtx 1 0 > results/gve/gve-$a-$g-t32.log 2>&1
done; done
