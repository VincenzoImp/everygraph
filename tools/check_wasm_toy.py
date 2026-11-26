#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The wasm toy check: the same toy kernel, native and wasm32, gives the same result.

tools/wasm-toy computes the degree of every node of a generated graph through the
public pieces of everygraph-core (`ByteSource`, `NodeId`, `par::map_ranges`), and runs
kernels of the edge-map framework (PageRank, the proof WCC, BFS; everygraph-kernels)
on the same graph over the raw and the compressed source, and hashes the results. This
script

1. builds and runs it natively, with the `parallel` feature (rayon), and
2. builds it for wasm32-unknown-unknown without threads, runs the module in Node,

and requires both to print the same lines, with the two sources agreeing (a kernels
hash of 0 means they did not). The toy also prints the golden digests of the core's
numerics (crates/everygraph-core/tests/numerics_golden.rs); the native run checks them
against the frozen values, so equal lines mean wasm32 has them too. Issue m3-11
(determinism CI) replaces the toy.

Usage: python3 tools/check_wasm_toy.py
       (from anywhere; needs cargo with the wasm32-unknown-unknown target, and node)
Exit status 0 if the two hashes are equal, 1 otherwise.
"""

import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOY = ROOT / "tools" / "wasm-toy"
TARGET_DIR = ROOT / "target" / "wasm-toy"
# Large enough for many chunks (the grain is 16384 edges), small enough for a second.
GRAPH = ["100000", "1000000", "42"]
ENV = {**os.environ, "CARGO_TARGET_DIR": str(TARGET_DIR)}
MANIFEST = ["--manifest-path", str(TOY / "Cargo.toml"), "--locked", "--release"]


def run(command):
    print("$ " + " ".join(command), flush=True)
    result = subprocess.run(command, cwd=ROOT, env=ENV, capture_output=True, text=True)
    if result.returncode != 0:
        print(result.stdout + result.stderr)
        raise SystemExit(f"FAILED: exit status {result.returncode}")
    return result.stdout.strip()


def main():
    native = run(["cargo", "run", *MANIFEST, "--features", "parallel", "--", *GRAPH])
    print(f"native: {native}")
    run(["cargo", "build", *MANIFEST, "--lib", "--target", "wasm32-unknown-unknown"])
    module = TARGET_DIR / "wasm32-unknown-unknown" / "release" / "everygraph_wasm_toy.wasm"
    wasm = run(["node", str(TOY / "run.mjs"), str(module), *GRAPH])
    print(f"wasm32: {wasm}")
    if native and native == wasm and "kernels-hash" in wasm and "hash=0000000000000000" not in wasm:
        print("ok: the native and the wasm32 runs agree")
        return 0
    print("FAILED: the native and the wasm32 runs differ")
    return 1


if __name__ == "__main__":
    sys.exit(main())
