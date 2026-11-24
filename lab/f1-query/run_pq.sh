#!/bin/bash
# Q2 point queries; whole machine under bench lock. args: graph names. COLD_ONLY=1 skips the warm run.
cd "$(dirname "$0")"
for gname in "$@"; do
  # private copy: fadvise(DONTNEED) cannot evict pages that other experiments keep mapped
  G=work/csr/$gname; [ -f $G/offsets.u64 ] || G=/home/tux/graphlab/data/csr/$gname
  [ -f $G/offsets.u64 ] || { echo "skip $gname"; continue; }
  [ -n "$COLD_ONLY" ] || flock /home/tux/graphlab/bench.lock ./target/release/pq --graph $G --name $gname --threads 32 --cold 0 \
     --out results/pq_${gname}_warm.json > /dev/null 2> logs/pq_${gname}_warm.err
  flock /home/tux/graphlab/bench.lock ./target/release/pq --graph $G --name $gname --threads 32 --warm 0 --prefetch 1 --random 1 \
     --out results/pq_${gname}_cold_pf_rand.json > /dev/null 2> logs/pq_${gname}_cold_pf_rand.err
  flock /home/tux/graphlab/bench.lock ./target/release/pq --graph $G --name $gname --threads 32 --warm 0 --prefetch 0 --random 0 \
     --out results/pq_${gname}_cold_plain.json > /dev/null 2> logs/pq_${gname}_cold_plain.err
  flock /home/tux/graphlab/bench.lock ./target/release/pq --graph $G --name $gname --threads 32 --warm 0 --prefetch 1 --random 0 \
     --out results/pq_${gname}_cold_pf_norm.json > /dev/null 2> logs/pq_${gname}_cold_pf_norm.err
done
