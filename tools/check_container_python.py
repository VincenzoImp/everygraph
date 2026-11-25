#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The container check: the independent Python reader reads what Rust wrote.

tools/eg_container.py is written from docs/spec/eg-container.md alone (#28). This script

1. writes a sample container with the Rust writer (`everygraph-io`'s example
   `container_sample`, two commits) and takes the listing the Rust reader prints;
2. lists it with the Python reader and requires the same commit and the same sections
   (name, kind, codec, offset, length, hash, provenance);
3. verifies every hash with the Python reader;
4. damages one byte of a section and requires the Python reader to name it;
5. damages the newest table of contents and requires the Python reader to fall back to
   the first commit, as the specification says.

Usage: python3 tools/check_container_python.py
       (from anywhere; needs cargo and python3, nothing else)
Exit status 0 if the check passes, 1 otherwise.
"""

import json
import os
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORK = ROOT / "target" / "container-python"
READER = ROOT / "tools" / "eg_container.py"


def run(command, ok=(0,)):
    print("$ " + " ".join(str(c) for c in command), flush=True)
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if result.returncode not in ok:
        print(result.stdout + result.stderr)
        raise SystemExit(f"FAILED: exit status {result.returncode}")
    return result


def python_reader(*args, ok=(0,)):
    return run([sys.executable, "-I", READER, *args], ok=ok)


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    sample = WORK / "sample.eg"
    rust = json.loads(
        run(["cargo", "run", "--quiet", "--locked", "-p", "everygraph-io",
             "--example", "container_sample", "--", sample]).stdout
    )
    listed = json.loads(python_reader("list", sample, "--json").stdout)
    if listed["generation"] != rust["generation"] or listed["sections"] != rust["sections"]:
        print("rust:  ", json.dumps(rust, indent=1))
        print("python:", json.dumps(listed, indent=1))
        print("FAILED: the Python reader lists different sections")
        return 1
    print(f"ok: {len(listed['sections'])} sections of commit {listed['generation']} listed alike")
    print(python_reader("verify", sample).stdout.strip())

    damaged = WORK / "damaged-section.eg"
    shutil.copy(sample, damaged)
    target = next(s for s in rust["sections"] if s["name"] == "x.many-chunks")
    with open(damaged, "r+b") as f:
        f.seek(target["offset"] + 40_000)
        byte = f.read(1)[0]
        f.seek(target["offset"] + 40_000)
        f.write(bytes([byte ^ 1]))
    out = python_reader("verify", damaged, ok=(1,)).stdout
    if "DAMAGED x.many-chunks" not in out or out.count("DAMAGED") != 1:
        print(out)
        print("FAILED: the damaged section was not the one reported")
        return 1
    print("ok: one flipped bit in x.many-chunks is reported, and only that section")

    damaged = WORK / "damaged-toc.eg"
    shutil.copy(sample, damaged)
    with open(damaged, "r+b") as f:
        size = os.path.getsize(damaged)
        f.seek(size - 4096 + 40)  # inside the newest TOC, the last item of the file
        byte = f.read(1)[0]
        f.seek(size - 4096 + 40)
        f.write(bytes([byte ^ 1]))
    result = python_reader("list", damaged, "--json")
    first = json.loads(result.stdout)
    names = {s["name"] for s in first["sections"]}
    if first["generation"] != 1 or "x.chunk-boundary" not in names or "warning" not in result.stderr:
        print(result.stdout + result.stderr)
        print("FAILED: the reader did not fall back to the first commit")
        return 1
    print("ok: with the newest table damaged, the reader falls back to commit 1 and says so")
    print(python_reader("verify", damaged).stdout.strip())
    shutil.rmtree(WORK)
    return 0


if __name__ == "__main__":
    sys.exit(main())
