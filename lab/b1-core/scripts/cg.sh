#!/bin/bash
# cg.sh LIMIT TIMEOUT_S LABEL cmd... : run under a hard cgroup memory limit with a cold page cache for the graph
cd /home/tux/graphlab/exp/b1-core
J=${J:-results/ooc_friendster.jsonl}; D=${D:-tmp/fr-a}
lim=$1; to=$2; label=$3; shift 3
bin/glab evict --graph $D
s=$(date +%s.%N)
systemd-run --user --scope -q -p MemoryMax=$lim -p MemorySwapMax=0 timeout $to "$@" --label "$label" --json $J
rc=$?
e=$(date +%s.%N)
echo "{\"cmd\":\"ooc-exit\",\"limit\":\"$lim\",\"label\":\"$label\",\"exit_code\":$rc,\"timeout_s\":$to,\"wall_seconds\":$(python3 -c "print(round($e-$s,2))")}" | tee -a $J
