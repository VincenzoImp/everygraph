#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Reproduces every number of lab/m2-35-random-read-probe/README.md: builds the probe,
# runs the full sweep (3 sizes x 5 queue depths x 4 variants x 3 interleaved repetitions)
# under a 4 GB memory limit, writes results/<tag>.jsonl, then prints the table.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
SCRATCH=${SCRATCH:-/home/tux/graphlab/scratch/m2-35}
FILE=${FILE:-${EVERYGRAPH_DATA:-/home/tux/graphlab/data}/ldbc-graphalytics/graphs/com-friendster/com-friendster.e}
TAG=${TAG:-exploratory-$(date +%Y%m%d)}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} CARGO_TARGET_DIR=$SCRATCH/target
(cd "$here" && cargo build --release --locked)
mkdir -p "$here/results"
uptime > "$here/results/$TAG-load.txt"
systemd-run --user --scope -q -p MemoryMax=4G -p MemorySwapMax=0 \
    "$CARGO_TARGET_DIR/release/rrprobe" --file "$FILE" --dev "${DEV:-nvme0n1}" "$@" \
    > "$here/results/$TAG.jsonl"
uptime >> "$here/results/$TAG-load.txt"
python3 -I "$here/summarize.py" "$here/results/$TAG.jsonl"
echo done
