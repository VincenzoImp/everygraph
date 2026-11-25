#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The oracle suite that runs on every pull request (issue #6). Budget: 10 minutes.
# Needs cargo and uv; no network beyond the first `uv sync`. Usage, from anywhere:
#
#   oracle/run-ci.sh            # work directory under $TMPDIR, removed at the end
#   ORACLE_WORK=dir oracle/run-ci.sh   # keep the corpus and references in `dir`
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
step() { printf '\n== %s (%s)\n' "$1" "$(date +%T)"; }
START=$SECONDS

step "Rust: unit, property-based, seeded-bug and golden tests"
cargo test -p everygraph-oracle --locked

step "Rust: build the oracle command"
cargo build -p everygraph-oracle --release --locked
ORACLE="$ROOT/target/release/everygraph-oracle"

step "Corpus: CI tier"
rm -rf "$WORK/ci"
"$ORACLE" corpus --tier ci --out "$WORK/ci"

step "Python: references (NetworkX, python-igraph, scipy)"
cd "$ROOT/oracle"
uv sync --locked --quiet
uv run --locked oracle-ref run "$WORK/ci" --jobs "$JOBS" --max-memory-gib "${ORACLE_MEMORY_GIB:-3}"
uv run --locked pytest -q
uv run --locked oracle-ref probe --check > /dev/null && echo "semantics probe unchanged"
uv run --locked oracle-ref golden --check
cd "$ROOT"

step "In-process references against the Python references"
"$ORACLE" check --corpus "$WORK/ci" --plugin reference --require-python

step "Toy kernel against everything"
"$ORACLE" check --corpus "$WORK/ci" --plugin toy --require-python

step "Seeded bugs, with the Python references"
"$ORACLE" seeded-bugs --corpus "$WORK/ci"

step "Golden files"
"$ORACLE" golden --dir "$ROOT/oracle/golden"

printf '\noracle CI suite passed in %d s\n' "$((SECONDS - START))"
