#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The wasm toy check: the same toy kernel, native and wasm32, gives the same result.

There is no real kernel yet. tools/wasm-toy computes the degree of every node of a
generated graph through the public pieces of everygraph-core (`ByteSource`, `NodeId`,
`par::map_ranges`) and hashes the result. This script

1. builds and runs it natively, with the `parallel` feature (rayon), and
2. builds it for wasm32-unknown-unknown without threads, runs the module in Node,

and requires both to print the same line. Issue m3-11 (determinism CI) replaces the
toy with the real kernels.

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
    if native and native == wasm:
        print("ok: the native and the wasm32 runs agree")
        return 0
    print("FAILED: the native and the wasm32 runs differ")
    return 1


if __name__ == "__main__":
    sys.exit(main())
