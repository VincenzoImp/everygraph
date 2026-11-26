#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The oracle suite that runs on every pull request (issue #6; job `oracle` in
# .github/workflows/ci.yml). Needs cargo and uv; no network after `uv sync`.
#
#   oracle/run-ci.sh                      # work directory under $TMPDIR, removed at the end
#   ORACLE_WORK=dir oracle/run-ci.sh      # keep the corpus and references in `dir`
#   ORACLE_SKIP_RUST_TESTS=1 oracle/run-ci.sh   # when `cargo test --workspace` already ran
#                                               # them (the CI `test` job does)
#
# On a shared machine run it under a memory limit, for example
#   systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 oracle/run-ci.sh
# The Python reference runs bound themselves too (--max-memory-gib per job).
#
# Each step prints its own summary; the script stops at the first failure.
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
JOBS="${ORACLE_JOBS:-2}"
MEM="${ORACLE_MEMORY_GIB:-3}"
step() { printf '\n== %s (%s, %d s)\n' "$1" "$(date +%T)" "$((SECONDS - START))"; }
START=$SECONDS

if [[ -z "${ORACLE_SKIP_RUST_TESTS:-}" ]]; then
    step "Rust: unit, property-based, seeded-bug and golden tests"
    cargo test -p everygraph-oracle --locked
fi

step "Rust: build the oracle and the dataset fetcher"
cargo build -p everygraph-oracle -p everygraph-bench --release --locked
ORACLE="$ROOT/target/release/everygraph-oracle"

step "Corpus: CI tier"
rm -rf "$WORK/ci"
"$ORACLE" corpus --tier ci --out "$WORK/ci"

step "Generated tier-1 datasets of the registry (no network)"
"$ROOT/target/release/everygraph-bench" fetch --data "$WORK/data" \
    synth-weighted-gnm-1000-5000 synth-bipartite-300x200 synth-path-100 synth-cycle-100 \
    synth-star-100 synth-complete-20 synth-grid-30x30 synth-barbell-10 synth-gnm-1000-5000 \
    > "$WORK/fetch.log" 2>&1 || { cat "$WORK/fetch.log"; exit 1; }
echo "9 generated datasets in $WORK/data"

step "Python: references (NetworkX, python-igraph, scipy), tests, semantics probe, golden files"
cd "$ROOT/oracle"
uv sync --locked --quiet
uv run --locked oracle-ref run "$WORK/ci" --jobs "$JOBS" --max-memory-gib "$MEM"
uv run --locked pytest -q
uv run --locked oracle-ref probe --check > /dev/null && echo "semantics probe unchanged"
uv run --locked oracle-ref golden --check --data-root "$WORK/data"
cd "$ROOT"

step "In-process references against the Python references"
"$ORACLE" check --corpus "$WORK/ci" --plugin reference --require-python

step "Toy kernel against everything"
"$ORACLE" check --corpus "$WORK/ci" --plugin toy --require-python

step "Edge-map kernels (#44, #47, #49) against everything, on every source and configuration"
EVERYGRAPH_ORACLE_CORPUS="$WORK/ci" cargo test -p everygraph-kernels --release --locked \
    --test oracle -- --nocapture

step "Seeded bugs, with the Python references"
"$ORACLE" seeded-bugs --corpus "$WORK/ci"

step "Golden files (karate and the generated tier-1 datasets)"
"$ORACLE" golden --dir "$ROOT/oracle/golden" --data-root "$WORK/data"

printf '\noracle CI suite passed in %d s\n' "$((SECONDS - START))"
