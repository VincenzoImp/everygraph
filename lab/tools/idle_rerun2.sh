#!/bin/bash
L=/home/tux/graphlab/logs
stamp() { echo "== $(date +%T) $1 (load $(cut -d' ' -f1 /proc/loadavg))" >> $L/idle_rerun.log; }
stamp "restart b2-leiden";  (cd /home/tux/graphlab/exp/b2-leiden && ./bench.sh) > $L/idle_b2.log 2>&1
stamp "start a-baselines";  (cd /home/tux/graphlab/exp/a-baselines && ./bench.sh) > $L/idle_a.log 2>&1
stamp "done"
