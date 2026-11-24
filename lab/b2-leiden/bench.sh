#!/usr/bin/env bash
# B2 — reproduces the headline numbers.   ./bench.sh   (lj + orkut; ~15 min on an idle machine)
#   GRAPHS="lj" THREADS="1 4 16 32" ./bench.sh   to restrict.   PIN=1 pins thread i -> cpu i (ONLY on an idle machine:
#   on the shared lab machine pinning onto cores used by other experiments made runs up to 10x slower).
set -euo pipefail
cd "$(dirname "$0")"
export B=./target/release/b2
export DATA=/home/tux/graphlab/data/csr
export LOCK=/home/tux/graphlab/bench.lock
GRAPHS=${GRAPHS:-"lj orkut"}
export THREADS=${THREADS:-"1 2 4 8 16 32"}
export PINFLAG=${PIN:+--pin}
nice -n 10 cargo build --release -j 6 --offline 2>/dev/null || nice -n 10 cargo build --release -j 6
mkdir -p results/scaling results/hier results/refs results/quality

for g in $GRAPHS; do
  export g
  # 1) thread scaling, async vs deterministic, Louvain vs Leiden (one lock acquisition per algo, to stay short)
  for algo in louvain leiden; do
    export algo
    flock "$LOCK" bash -c '
      for mode in async det; do for t in $THREADS; do
        extra=""; [ "$mode" = det ] && extra="--det"
        rep=2; [ "$t" -le 2 ] && rep=1
        $B run --graph $DATA/$g --algo $algo --threads $t $PINFLAG --repeat $rep $extra \
           --json results/scaling/${g}_${algo}_${mode}_t$t.json > /dev/null
      done; done'
  done
  # 2) quality mode (3 Leiden iterations) + hierarchy variants, 16 threads
  flock "$LOCK" bash -c '
    $B run --graph $DATA/$g --algo leiden --threads 16 $PINFLAG --iterations 3 --json results/quality/${g}_leiden_async_it3.json > /dev/null
    $B run --graph $DATA/$g --algo leiden --threads 16 $PINFLAG --iterations 3 --det --json results/quality/${g}_leiden_det_it3.json > /dev/null
    $B run --graph $DATA/$g --algo leiden --threads 32 $PINFLAG --iterations 3 --det --json results/quality/${g}_leiden_det_it3_t32.json > /dev/null
    $B run --graph $DATA/$g --algo leiden --threads 16 $PINFLAG --natural --json results/quality/${g}_leiden_async_naturalorder.json > /dev/null
    $B run --graph $DATA/$g --algo louvain --threads 16 $PINFLAG --natural --json results/quality/${g}_louvain_async_naturalorder.json > /dev/null
    $B run --graph $DATA/$g --algo leiden --threads 16 $PINFLAG --min-ratio 3 --json results/hier/${g}_leiden_natural_ratio3.json > /dev/null
    $B run --graph $DATA/$g --algo louvain --threads 16 $PINFLAG --json results/hier/${g}_louvain_natural.json > /dev/null
    $B run --graph $DATA/$g --algo leiden --threads 16 $PINFLAG --cap 64 --max-passes 20 --min-ratio 3 --json results/hier/${g}_leiden_cap64_ratio3.json > /dev/null
    $B run --graph $DATA/$g --algo leiden --threads 16 $PINFLAG --cap 8 --max-passes 30 --min-ratio 6 --json results/hier/${g}_leiden_cap8_ratio6.json > /dev/null
    $B run --graph $DATA/$g --algo leiden --threads 16 $PINFLAG --det --cap 64 --max-passes 20 --min-ratio 3 --json results/hier/${g}_leiden_det_cap64_ratio3.json > /dev/null
    $B run --graph $DATA/$g --algo louvain --threads 16 $PINFLAG --cap 32 --max-passes 20 --min-ratio 3 --json results/hier/${g}_louvain_cap32_ratio3.json > /dev/null'
  # 3) reference partitions (if ref/out exists; produced by ref/refs.py — slow, not re-run here) through the common eval
  for f in ref/out/$g.*.u32; do
    [ -e "$f" ] || continue
    a=$(basename "$f" .u32); a=${a#$g.}
    $B eval --graph $DATA/$g --threads 6 --memb "$f" > results/refs/${g}_$a.json
  done
done
python3 summarize.py > results/summary.txt
cat results/summary.txt
