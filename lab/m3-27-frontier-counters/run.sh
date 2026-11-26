#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Reproduces every count of lab/m3-27-frontier-counters/README.md. Counts of work, not
# times: the machine may be loaded. Scratch (build, stores, raw call logs) goes under
# $SCRATCH; results/ gets one JSON line per run and the call logs, gzipped.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
data=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}/csr
SCRATCH=${SCRATCH:-/home/tux/graphlab/scratch/m3-27}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-6} CARGO_TARGET_DIR=$SCRATCH/target-lab
(cd "$here" && cargo build --release --locked)
fc=$CARGO_TARGET_DIR/release/fc
res=$here/results
mkdir -p "$res/calls" "$SCRATCH/stores" "$SCRATCH/calls"
limit() { case $1 in friendster) echo 24G ;; *) echo 16G ;; esac; }
lim() { systemd-run --user --scope -q -p MemoryMax="$(limit "$1")" -p MemorySwapMax=0 "${@:2}"; }
one() { # graph kernel threads tag [extra args]
    local g=$1 k=$2 t=$3 tag=$4
    local calls=$SCRATCH/calls/$g-$k-t$t-$tag.jsonl
    lim "$g" "$fc" run --graph "$data/$g" --kernel "$k" --threads "$t" --calls "$calls" "${@:5}" \
        | sed "s/^{/{\"run\":\"$tag\",/" >> "$res/runs.jsonl"
    gzip -9 -c "$calls" > "$res/calls/$g-$k-t$t-$tag.jsonl.gz"
}
: > "$res/runs.jsonl"
# The copy of SSSP against the product kernel, at one thread: identical step logs.
one orkut sssp-product 1 a
one orkut sssp 1 a
for g in orkut twitter-sym friendster; do
    for rep in a b; do
        one "$g" kcore 16 "$rep"
        one "$g" sssp 16 "$rep"
    done
    [ -f "$SCRATCH/stores/$g/adjacency.data" ] \
        || lim "$g" "$fc" encode --graph "$data/$g" --out "$SCRATCH/stores/$g"
    one "$g" bfs 16 a --store "$SCRATCH/stores/$g"
    one "$g" wcc 16 a --store "$SCRATCH/stores/$g"
done
# k-core at one thread: no race in the local queues, so a reference for the spread.
one orkut kcore 1 a
# Leiden's own driver over the frontier driver (deterministic mode, default options).
# twitter-sym is killed at 16 GB (exit 137), so orkut only.
one orkut leiden 16 a
echo done
