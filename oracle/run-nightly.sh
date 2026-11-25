#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The nightly oracle suite (issue #6): the CI suite at larger scale, the golden files of
# the downloaded SNAP graphs, and the lab prototype through its adapter. Needs network
# (SNAP) and, for the lab step, a toolchain that builds lab/b1-core.
#
#   oracle/run-nightly.sh               # work directory under $TMPDIR, removed at the end
#   ORACLE_WORK=dir oracle/run-nightly.sh
#   ORACLE_SKIP_LAB=1 oracle/run-nightly.sh
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
JOBS="${ORACLE_JOBS:-8}"
step() { printf '\n== %s (%s)\n' "$1" "$(date +%T)"; }
START=$SECONDS

step "Rust: property-based tests with 2,000 cases each"
PROPTEST_CASES=2000 cargo test -p everygraph-oracle --release --locked

cargo build -p everygraph-oracle --release --locked
ORACLE="$ROOT/target/release/everygraph-oracle"

step "Corpus: nightly tier"
rm -rf "$WORK/nightly"
"$ORACLE" corpus --tier nightly --out "$WORK/nightly"

step "Python: references"
cd "$ROOT/oracle"
uv sync --locked --quiet
uv run --locked oracle-ref run "$WORK/nightly" --jobs "$JOBS"

step "Golden files of the SNAP graphs (downloads, checksums verified)"
uv run --locked oracle-ref golden --fetch
uv run --locked oracle-ref golden --check
cd "$ROOT"
"$ORACLE" golden --dir "$ROOT/oracle/golden"

step "In-process references, toy kernel and seeded bugs on the nightly corpus"
"$ORACLE" check --corpus "$WORK/nightly" --plugin reference --require-python
"$ORACLE" check --corpus "$WORK/nightly" --plugin toy --require-python
"$ORACLE" seeded-bugs --corpus "$WORK/nightly"

if [[ -z "${ORACLE_SKIP_LAB:-}" ]]; then
    step "Lab prototype B1 through its adapter (undirected graphs; 3 regimes x 2 thread counts x 2 chunk sizes)"
    (cd lab/b1-core && cargo build --release -p glab-oracle)
    TMPDIR="$WORK" "$ORACLE" check --corpus "$WORK/nightly" --plugin external \
        --program "$ROOT/lab/b1-core/target/release/glab-oracle" \
        --kernels adjacency,degree,pagerank,wcc,bfs,core_number,triangle_total \
        --threads 1,4 --chunks 64,65536 --regimes in_memory,mmap,stream --require-python
fi

printf '\noracle nightly suite passed in %d s\n' "$((SECONDS - START))"
