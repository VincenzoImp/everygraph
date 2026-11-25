#!/bin/bash
# Official Friendster tracer run (issue #15), as used on 2026-11-25. Run it detached:
#   setsid nohup bash lab/m1-01-tracer/run_official.sh > launcher.log 2>&1 &
# One line per finished row in $S/stages.log, so the state is visible if the session ends.
# Waits for the 1-minute load average only (the harness gate checks the same reading).
# Needs a release build of everygraph-bench in the checkout's target/.
set -u
S=${EVERYGRAPH_SCRATCH:-/home/tux/graphlab/scratch/m1-01-tracer}
R=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$S"
export EVERYGRAPH_DATA=${EVERYGRAPH_DATA:-/home/tux/graphlab/data} EVERYGRAPH_SCRATCH=$S
cd "$R"
st() { echo "$(date -u +%FT%TZ) $*" >> $S/stages.log; }
st "launcher started (pid $$), waiting for 1-min load <= 0.9"
while awk '{exit !($1 > 0.9)}' /proc/loadavg; do sleep 10; done
st "load $(cut -d' ' -f1-3 /proc/loadavg); starting official run"
./target/release/everygraph-bench run tracer --official --only graph=friendster > $S/official-run.log 2>&1 &
RUN=$!
st "harness pid $RUN"
seen=0
report() {
  mapfile -t rows < <(grep -E '^\[[0-9]+/[0-9]+\]' $S/official-run.log)
  for ((i = seen; i < ${#rows[@]}; i++)); do
    l=${rows[$i]}
    case "$l" in *"  ok") st "done   $l" ;; *) st "failed $l" ;; esac
  done
  seen=${#rows[@]}
}
while kill -0 $RUN 2>/dev/null; do report; sleep 10; done
wait $RUN; rc=$?
report
st "official run exited $rc"
mkdir -p lab/m1-01-tracer/results
python3 -I lab/m1-01-tracer/verify.py --store $S/bench-work/tracer/store --graph friendster \
  --out lab/m1-01-tracer/results/verify_friendster.json > $S/verify.log 2>&1
st "verify exited $?"
st "finished"
