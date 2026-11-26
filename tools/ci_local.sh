#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The Linux x86_64 part of CI, run locally: the commands of CONTRIBUTING.md "Checks".
# A pre-push check, and evidence to attach to a pull request when GitHub Actions cannot
# run. It does not replace CI: Windows, macOS and aarch64 are not covered.
#
#   tools/ci_local.sh [--miri] [--oracle]     (run it under a hard memory limit:
#   systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0 tools/ci_local.sh)
set -u
cd "$(dirname "$0")/.."
miri=0; oracle=0
for a in "$@"; do case $a in --miri) miri=1 ;; --oracle) oracle=1 ;; esac; done
log=$(mktemp -d)/ci_local.log
pass=0; fail=0; failed=()
step() {
  local name=$1; shift
  if "$@" >>"$log" 2>&1; then pass=$((pass + 1)); else fail=$((fail + 1)); failed+=("$name"); fi
}
step fmt cargo fmt --all --check
step clippy cargo clippy --workspace --all-targets -- -D warnings
step clippy-core-nodefault cargo clippy -p everygraph-core --no-default-features --all-targets -- -D warnings
step clippy-core-i686 cargo clippy -p everygraph-core --target i686-unknown-linux-gnu --all-targets -- -D warnings
step clippy-core-i686-nodefault cargo clippy -p everygraph-core --target i686-unknown-linux-gnu --no-default-features --all-targets -- -D warnings
step portability python3 tools/check_core_portability.py
step numerics-lint python3 tools/numerics_lint.py --check
step nextest cargo nextest run --workspace
step doctest cargo test --workspace --doc
step test-core-nodefault cargo test -p everygraph-core --no-default-features
step wasm-core cargo build -p everygraph-core --target wasm32-unknown-unknown --no-default-features
step wasm-ingest cargo build -p everygraph-ingest --target wasm32-unknown-unknown
step wasm-crate cargo build -p everygraph-wasm --target wasm32-unknown-unknown
step clippy-format-i686 cargo clippy -p everygraph-format --target i686-unknown-linux-gnu --all-targets -- -D warnings
step clippy-kernels-i686 cargo clippy -p everygraph-kernels --target i686-unknown-linux-gnu --all-targets -- -D warnings
step clippy-format-wasm cargo clippy -p everygraph-format --target wasm32-unknown-unknown -- -D warnings
step clippy-format-simd128 env RUSTFLAGS="-C target-feature=+simd128" CARGO_TARGET_DIR=target/wasm-simd128 \
  cargo clippy -p everygraph-format --target wasm32-unknown-unknown -- -D warnings
step deny cargo deny check
step doc env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
step canary python3 tools/check_portability_canary.py
step fuzz-replay tools/fuzz.sh replay
step wasm-toy python3 tools/check_wasm_toy.py
step wasm-codec python3 tools/check_wasm_codec.py
step container python3 tools/check_container_python.py
step columns-python uv run -q --script tools/check_columns_python.py
step python-package tools/check_python_package.sh
# While 1.100 is the current stable, the nextest and doctest steps already ran on it.
if [ "$(rustc +1.100 -V)" != "$(rustc -V)" ]; then step msrv cargo +1.100 test --workspace --locked; fi
if [ $miri = 1 ]; then
  step miri-core cargo +nightly miri test -p everygraph-core --no-default-features -- --skip full_size
  step miri-format cargo +nightly miri test -p everygraph-format
  step tsan python3 tools/check_tsan.py
fi
[ $oracle = 1 ] && step oracle oracle/run-ci.sh
echo "ci_local $(git rev-parse --short HEAD): $pass passed, $fail failed${failed:+: ${failed[*]}} (log $log)"
[ $fail = 0 ]
