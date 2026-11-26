#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The container check: the independent Python reader reads what Rust wrote.

tools/eg_container.py is written from docs/spec/eg-container.md alone (#28). This script
has `everygraph-io`'s example `container_sample` write a sample container (two
commits: block 0 holds generation 1, block 1 generation 2), then makes files from it
that exercise the reading rules of § 7, and for each requires the Python reader and the
Rust reader (`container_sample list`) to report the same commit, the same fallback and
the same sections (name, kind, codec, offset, length, hash, provenance) and the same
graph identity (§ 6):

- the sample itself, whose hashes the Python reader must also verify;
- the newest table damaged (its hash fails): both fall back to commit 1;
- both header blocks at the same generation: block 0 wins;
- an unknown required feature bit on the *older* block only: commit 2 opens;
- the reserved `segments` bit (0) on the newest block, and a newest block of a later
  major version: both refuse the file;
- a `seg/` name in the newest table without the segments bit: both fall back;
- the newest table re-hashed after breaking a rule (kind 0), and after giving a name
  bytes that are not UTF-8: both fall back to commit 1;

and, with the Python reader alone, that one flipped bit in a section is reported as
that section and nothing else.

Usage: python3 tools/check_container_python.py
       (from anywhere; needs cargo and python3, nothing else)
Exit status 0 if the check passes, 1 otherwise.
"""

import importlib.util
import json
import pathlib
import shutil
import struct
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORK = ROOT / "target" / "container-python"
READER = ROOT / "tools" / "eg_container.py"
SAMPLE = ["cargo", "run", "--quiet", "--locked", "-p", "everygraph-io", "--example", "container_sample", "--"]

spec = importlib.util.spec_from_file_location("eg_container", READER)
eg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(eg)


def run(command, ok=(0,)):
    print("$ " + " ".join(str(c) for c in command), flush=True)
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if result.returncode not in ok:
        print(result.stdout + result.stderr)
        raise SystemExit(f"FAILED: exit status {result.returncode}")
    return result


def python_reader(*args, ok=(0,)):
    return run([sys.executable, "-I", READER, *args], ok=ok)


def compare(path, expect_generation, expect_recovered, what):
    """The two readers agree on `path`, and on what is expected."""
    rust = json.loads(run([*SAMPLE, "list", path]).stdout)
    python = json.loads(python_reader("list", path, "--json").stdout)
    keys = ("generation", "recovered", "identity", "sections")
    same = all(rust.get(k) == python.get(k) for k in keys)
    expected = rust.get("generation") == expect_generation and rust.get("recovered") == expect_recovered
    if not (same and expected):
        print("rust:  ", json.dumps(rust, indent=1))
        print("python:", json.dumps(python, indent=1))
        raise SystemExit(f"FAILED: {what}")
    print(f"ok: {what} — both read commit {rust['generation']}, {len(rust['sections'])} sections")
    return rust


def patch_header(data, block, edit):
    b = bytearray(data[block * 4096:block * 4096 + 128])
    edit(b)
    b[96:128] = eg.blake3(bytes(b[:96]))
    data[block * 4096:block * 4096 + 128] = b


def patch_newest_toc(data, edit):
    """Edits the table of block 1 (generation 2) and fixes its hash and the header's."""
    toc_offset, toc_len = struct.unpack_from("<QQ", data, 4096 + 48)
    toc = bytearray(data[toc_offset:toc_offset + toc_len])
    edit(toc)
    data[toc_offset:toc_offset + toc_len] = toc
    patch_header(data, 1, lambda b: b.__setitem__(slice(64, 96), eg.blake3(bytes(toc))))


def fixture(sample, name, edit):
    data = bytearray(sample.read_bytes())
    edit(data)
    path = WORK / name
    path.write_bytes(bytes(data))
    return path


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    sample = WORK / "sample.eg"
    run([*SAMPLE, "write", sample])
    rust = compare(sample, 2, False, "the sample")
    print(python_reader("verify", sample).stdout.strip())

    target = next(s for s in rust["sections"] if s["name"] == "x.many-chunks")
    path = fixture(sample, "damaged-section.eg", lambda d: d.__setitem__(target["offset"] + 40_000, d[target["offset"] + 40_000] ^ 1))
    out = python_reader("verify", path, ok=(1,)).stdout
    if "DAMAGED x.many-chunks" not in out or out.count("DAMAGED") != 1:
        print(out)
        raise SystemExit("FAILED: the damaged section was not the one reported")
    print("ok: one flipped bit in x.many-chunks is reported, and only that section")

    toc_offset = struct.unpack_from("<Q", sample.read_bytes(), 4096 + 48)[0]
    path = fixture(sample, "damaged-toc.eg", lambda d: d.__setitem__(toc_offset + 40, d[toc_offset + 40] ^ 1))
    first = compare(path, 1, True, "the newest table damaged: fall back")
    if "x.chunk-boundary" not in {s["name"] for s in first["sections"]}:
        raise SystemExit("FAILED: the fallback is not commit 1")
    print(python_reader("verify", path).stdout.strip())

    path = fixture(sample, "tie.eg", lambda d: patch_header(d, 0, lambda b: b.__setitem__(slice(24, 32), struct.pack("<Q", 2))))
    tie = compare(path, 2, False, "both blocks at generation 2: block 0 wins")
    if tie["sections"] != first["sections"]:
        raise SystemExit("FAILED: on a tie, block 1's table was used")

    path = fixture(sample, "required-older.eg", lambda d: patch_header(d, 0, lambda b: b.__setitem__(slice(32, 40), struct.pack("<Q", 1))))
    compare(path, 2, False, "an unknown required feature on the older block only")

    # Required bit 0, `segments`, reserved (§ 10): both readers refuse the file.
    path = fixture(sample, "segments.eg", lambda d: patch_header(d, 1, lambda b: b.__setitem__(slice(32, 40), struct.pack("<Q", 1))))
    rust = json.loads(run([*SAMPLE, "list", path]).stdout)
    python = python_reader("list", path, "--json", ok=(1,))
    if "segments" not in rust.get("error", "") or "features" not in python.stderr:
        print(rust, python.stdout + python.stderr)
        raise SystemExit("FAILED: a file with the segments bit was not refused by both readers")
    print("ok: required bit 0 (segments) on the newest block — both readers refuse the file")

    # A foreign newest block (major version 2) refuses the file in both readers (§ 7).
    path = fixture(sample, "foreign.eg", lambda d: patch_header(d, 1, lambda b: b.__setitem__(slice(12, 14), struct.pack("<H", 2))))
    rust = json.loads(run([*SAMPLE, "list", path]).stdout)
    python = python_reader("list", path, "--json", ok=(1,))
    if "version 2" not in rust.get("error", "") or "version 2" not in python.stderr:
        print(rust, python.stdout + python.stderr)
        raise SystemExit("FAILED: a foreign newest block was not refused by both readers")
    print("ok: a newest block of major version 2 — both readers refuse the file")

    def seg_name(toc):  # "x.many-chunks" → "seg/manychunk": same length, same order
        at = bytes(toc).rindex(b"x.many-chunks")
        toc[at:at + 13] = b"seg/manychunk"
    path = fixture(sample, "seg-name.eg", lambda d: patch_newest_toc(d, seg_name))
    compare(path, 1, True, "a seg/ name without the segments bit: fall back")

    path = fixture(sample, "kind-zero.eg", lambda d: patch_newest_toc(d, lambda t: t.__setitem__(slice(32, 36), bytes(4))))
    compare(path, 1, True, "the newest table breaks a rule (kind 0): fall back")

    def bad_name(toc):
        n, size = struct.unpack_from("<II", toc, 8)
        toc[32 + n * size] = 0xFF  # first byte of the first name
    path = fixture(sample, "bad-name.eg", lambda d: patch_newest_toc(d, bad_name))
    compare(path, 1, True, "a name in the newest table is not UTF-8: fall back")

    shutil.rmtree(WORK)
    return 0


if __name__ == "__main__":
    sys.exit(main())
