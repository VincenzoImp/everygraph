#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Experiment m1-20 (issue #177): Everygraph's lab kernels and the oracle against the LDBC
# Graphalytics reference outputs. Writes one JSON file per step into results/.
# These are not LDBC Benchmark Results. Times in the files are exploratory.
#
#   lab/m1-20-ldbc-graphalytics/run.sh [small|s-class|friendster|all]
#
# Environment: LDBC_DATA (registry data root, default $EVERYGRAPH_DATA/ldbc-graphalytics),
# LDBC_GRAPHS (unpacked graphs, default $LDBC_DATA/graphs), SCRATCH (CSR files of
# com-friendster, about 15.5 GB, default /home/tux/graphlab/scratch/m1-20-ldbc-graphalytics).
#
# Memory limits: 8 GB for everything except building the com-friendster CSR (22 GB:
# its symmetric lists alone are 14.4 GB) and the in-RAM friendster check (20 GB).
# The "stream" friendster check runs under 8 GB, below the 14.4 GB of lists.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
DATA="${LDBC_DATA:-${EVERYGRAPH_DATA:-/home/tux/graphlab/data}/ldbc-graphalytics}"
GRAPHS="${LDBC_GRAPHS:-$DATA/graphs}"
SCRATCH="${SCRATCH:-/home/tux/graphlab/scratch/m1-20-ldbc-graphalytics}"
OUT="$HERE/results"
export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-8}"
limit() { local m="$1"; shift; systemd-run --user --scope -q -p MemoryMax="$m" -p MemorySwapMax=0 "$@"; }

mkdir -p "$OUT" "$SCRATCH"
(cd "$ROOT" && cargo build --release --locked -p everygraph-oracle -p everygraph-bench)
(cd "$HERE" && cargo build --release)
ORACLE="$ROOT/target/release/everygraph-oracle"
LAB="$HERE/target/release/ldbc-lab"
. "$ROOT/oracle/ldbc-unpack.sh"

what="${1:-all}"
if [[ $what == small || $what == all ]]; then
    ldbc_unpack "$ROOT/target/release/everygraph-bench" "$DATA" "$GRAPHS" ldbc-test
    mkdir -p "$SCRATCH/small"
    for d in "$GRAPHS"/example-* "$GRAPHS"/test-*; do ln -sfn "$d" "$SCRATCH/small/"; done
    limit 8G "$ORACLE" ldbc --graphs "$SCRATCH/small" --expect "$ROOT/oracle/ldbc-expected.json" \
        --json "$OUT/oracle-test-graphs.json"
    for d in "$SCRATCH"/small/*; do
        limit 8G "$LAB" check --graph "$d" --spec bfs,pr,wcc,cdlp,lcc,sssp \
            --json "$OUT/lab-$(basename "$d").json"
    done
fi
if [[ $what == s-class || $what == all ]]; then
    ldbc_unpack "$ROOT/target/release/everygraph-bench" "$DATA" "$GRAPHS" ldbc-graph500-22 ldbc-dota-league
    mkdir -p "$SCRATCH/s-class"
    ln -sfn "$GRAPHS/graph500-22" "$GRAPHS/dota-league" "$SCRATCH/s-class/"
    limit 8G "$ORACLE" ldbc --graphs "$SCRATCH/s-class" --implementations spec,first-seen \
        --expect "$ROOT/oracle/ldbc-expected.json" --max-memory 7 --json "$OUT/oracle-s-class.json"
    for g in graph500-22 dota-league; do
        limit 8G "$LAB" check --graph "$GRAPHS/$g" --json "$OUT/lab-$g.json"
    done
fi
if [[ $what == friendster || $what == all ]]; then
    ldbc_unpack "$ROOT/target/release/everygraph-bench" "$DATA" "$GRAPHS" ldbc-com-friendster
    FR="$GRAPHS/com-friendster"
    CSR="$SCRATCH/csr-friendster"
    [[ -f $CSR/targets.u32 ]] || limit 22G "$LAB" build --graph "$FR" --out "$CSR"
    # In RAM: CSR files read into the heap; the specification's BFS, WCC, PageRank and
    # CDLP beside the lab kernels.
    limit 20G "$LAB" check --graph "$FR" --csr "$CSR" --load heap --spec bfs,wcc,pr,cdlp \
        --json "$OUT/lab-com-friendster-heap.json"
    # Under a budget below the lists' 14.4 GB: semi-external PageRank and WCC, BFS on a map.
    # Rows carry a hash of the result bits, to compare with the heap run.
    # Not run: the specification's sequential BFS on the map under 8 GB. It visits lists in
    # queue order and read 175 GB in 10 minutes without finishing (killed).
    limit 8G "$LAB" check --graph "$FR" --csr "$CSR" --load stream \
        --json "$OUT/lab-com-friendster-stream-8g.json"
fi
