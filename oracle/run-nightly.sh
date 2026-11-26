#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The nightly oracle suite (issue #6): the CI suite at larger scale, the golden files of
# every tier-1 dataset of the registry and of the extra SNAP graphs, the LDBC Graphalytics
# graphs of the S class (issue #177), and the lab prototype through its adapter. Needs
# network (SNAP, Wikimedia, LDBC) and, for the lab step, a toolchain that builds
# lab/b1-core.
#
#   oracle/run-nightly.sh               # work directory under $TMPDIR, removed at the end
#   ORACLE_WORK=dir oracle/run-nightly.sh
#   ORACLE_DATA=dir oracle/run-nightly.sh   # reuse a data root already holding tier 1
#   ORACLE_SKIP_LAB=1 oracle/run-nightly.sh
#
# On a shared machine run it under a memory limit:
#   systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 oracle/run-nightly.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
if [[ -n "${ORACLE_WORK:-}" ]]; then
    WORK="$ORACLE_WORK"
    mkdir -p "$WORK"
else
    WORK="$(mktemp -d)"
    trap 'rm -rf "$WORK"' EXIT
fi
DATA="${ORACLE_DATA:-$WORK/data}"
JOBS="${ORACLE_JOBS:-2}"
MEM="${ORACLE_MEMORY_GIB:-3}"
step() { printf '\n== %s (%s, %d s)\n' "$1" "$(date +%T)" "$((SECONDS - START))"; }
START=$SECONDS

step "Rust: property-based tests with 2,000 cases each"
PROPTEST_CASES=2000 cargo test -p everygraph-oracle --release --locked

cargo build -p everygraph-oracle -p everygraph-bench --release --locked
ORACLE="$ROOT/target/release/everygraph-oracle"

step "Corpus: nightly tier"
rm -rf "$WORK/nightly"
"$ORACLE" corpus --tier nightly --out "$WORK/nightly"

step "Python: references"
cd "$ROOT/oracle"
uv sync --locked --quiet
uv run --locked oracle-ref run "$WORK/nightly" --jobs "$JOBS" --max-memory-gib "$MEM"

step "Golden files: tier 1 of the registry and the extra SNAP graphs (downloads, checksums verified)"
"$ROOT/target/release/everygraph-bench" fetch tier1 --data "$DATA"
uv run --locked oracle-ref golden --fetch
uv run --locked oracle-ref golden --check --data-root "$DATA"
cd "$ROOT"
"$ORACLE" golden --dir "$ROOT/oracle/golden" --data-root "$DATA" --require-all

step "In-process references, toy kernel and seeded bugs on the nightly corpus"
"$ORACLE" check --corpus "$WORK/nightly" --plugin reference --require-python
"$ORACLE" check --corpus "$WORK/nightly" --plugin toy --require-python
"$ORACLE" seeded-bugs --corpus "$WORK/nightly"

step "LDBC Graphalytics: test graphs, graph500-22 and dota-league against their reference outputs (330 MB)"
. "$ROOT/oracle/ldbc-unpack.sh"
ldbc_unpack "$ROOT/target/release/everygraph-bench" "$DATA" "$WORK/ldbc" \
    ldbc-test ldbc-graph500-22 ldbc-dota-league
# graph500-22 needs about 3 GiB; LCC on dota-league (50 M edges on 61 k vertices) takes
# a few minutes, sequential.
"$ORACLE" ldbc --graphs "$WORK/ldbc" --expect "$ROOT/oracle/ldbc-expected.json" --max-memory 7
EVERYGRAPH_LDBC_GRAPHS="$WORK/ldbc" cargo test -p everygraph-kernels --release --locked \
    --test ldbc -- --nocapture

if [[ -z "${ORACLE_SKIP_LAB:-}" ]]; then
    step "Lab prototype B1 through its adapter (undirected graphs; 3 regimes x 2 thread counts x 2 chunk sizes)"
    (cd lab/b1-core && cargo build --release -p glab-oracle)
    # Reported, not gating: the prototype is research code that will not be fixed. Its
    # known finding (PR #131): PageRank's dangling-mass sum follows the chunking, so
    # results differ in the last bits between chunk sizes on graphs with isolated nodes.
    TMPDIR="$WORK" "$ORACLE" check --corpus "$WORK/nightly" --plugin external \
        --program "$ROOT/lab/b1-core/target/release/glab-oracle" \
        --kernels adjacency,degree,pagerank,wcc,bfs,core_number,triangle_total \
        --threads 1,2 --chunks 64,65536 --regimes in_memory,mmap,stream --require-python \
        || echo "lab prototype: findings above (reported, not gating)"
fi

printf '\noracle nightly suite passed in %d s\n' "$((SECONDS - START))"
