#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The wasm codec check: the simd128 adjacency decoder agrees with the scalar one.

The `simd128` decoder of everygraph-format exists only in a wasm32 build with that
target feature, so no native test reaches it. tools/wasm-codec compares every decoder
of the build with the scalar one, on every reading API, over generated graphs (clean and
with damaged bytes). This script

1. builds and runs it natively (x86-64: scalar against SSSE3), and
2. builds it for wasm32-unknown-unknown with `-C target-feature=+simd128`, runs the
   module in Node (scalar against simd128),

and requires both lines to be equal, with two decoders and no mismatch.

Usage: python3 tools/check_wasm_codec.py
       (from anywhere; needs cargo with the wasm32-unknown-unknown target, and node)
Exit status 0 if the check passes, 1 otherwise.
"""

import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOOL = ROOT / "tools" / "wasm-codec"
TARGET_DIR = ROOT / "target" / "wasm-codec"
# About ten seconds in Node: 3 graphs of 20,000 nodes, one list longer than a block.
RUN = ["20261125", "3", "20000"]
MANIFEST = ["--manifest-path", str(TOOL / "Cargo.toml"), "--locked", "--release"]


def run(command, extra_env=None):
    print("$ " + " ".join(command), flush=True)
    env = {**os.environ, "CARGO_TARGET_DIR": str(TARGET_DIR), **(extra_env or {})}
    result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
    if result.returncode != 0:
        print(result.stdout + result.stderr)
        raise SystemExit(f"FAILED: exit status {result.returncode}")
    return result.stdout.strip()


def main():
    native = run(["cargo", "run", *MANIFEST, "--", *RUN])
    print(f"native: {native}")
    simd128 = {"RUSTFLAGS": "-C target-feature=+simd128"}
    run(["cargo", "build", *MANIFEST, "--lib", "--target", "wasm32-unknown-unknown"], simd128)
    module = TARGET_DIR / "wasm32-unknown-unknown" / "release" / "everygraph_wasm_codec.wasm"
    wasm = run(["node", str(TOOL / "run.mjs"), str(module), *RUN])
    print(f"wasm32: {wasm}")
    if native == wasm and " decoders=2 mismatches=0 " in wasm:
        print("ok: simd128 and the native SIMD decoder agree with the scalar decoder")
        return 0
    print("FAILED: a decoder disagrees, a decoder is missing, or the runs differ")
    return 1


if __name__ == "__main__":
    sys.exit(main())
