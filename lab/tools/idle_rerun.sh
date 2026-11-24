#!/bin/bash
# Idle-machine re-run of the headline benchmarks. Holds both lab locks for the whole chain.
L=/home/tux/graphlab/logs; mkdir -p $L
stamp() { echo "== $(date +%T) $1 (load $(cut -d' ' -f1 /proc/loadavg))" >> $L/idle_rerun.log; }
stamp "start b1-core";      (cd /home/tux/graphlab/exp/b1-core && ./bench.sh) > $L/idle_b1.log 2>&1
stamp "start b2-leiden";    (cd /home/tux/graphlab/exp/b2-leiden && ./bench.sh) > $L/idle_b2.log 2>&1
stamp "start a-baselines";  (cd /home/tux/graphlab/exp/a-baselines && ./bench.sh) > $L/idle_a.log 2>&1
stamp "done"
