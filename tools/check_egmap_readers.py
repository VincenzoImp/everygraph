#!/usr/bin/env -S uv run --script
# SPDX-License-Identifier: Apache-2.0 OR MIT
# /// script
# requires-python = ">=3.11"
# dependencies = ["pmtiles==3.8.1"]
# ///
"""The map file check: three readers agree on what the Rust writer wrote (#68).

`everygraph-map`'s example `egmap_sample` writes a sample map. Then:

1. The independent Python reader (`tools/egmap.py`, written from `docs/spec/egmap.md`
   alone) and the Rust reader dump the whole file; the dumps must be equal, and the
   census must find no popping.
2. The official PMTiles reader for Python (`pmtiles` on PyPI) opens the file, reads the
   header and the metadata, and fetches every tile by `(z, x, y)`; each tile must decode,
   with the Python reader, to the tile of the dumps.
3. The official PMTiles reader for JavaScript (`pmtiles` on npm, in Node) does the same,
   when `node` and `npm` are on the path (CI has them); without them this step is
   reported as skipped.
4. Damaged copies (a run length of 2, header counts that disagree, a cut file, a section
   table with another magic) are refused by both the Python and the Rust reader.

Usage: uv run --script tools/check_egmap_readers.py
Exit status 0 if the check passes, 1 otherwise.
"""

import gzip
import hashlib
import importlib.util
import json
import pathlib
import shutil
import struct
import subprocess
import sys

from pmtiles.reader import MmapSource, Reader

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORK = ROOT / "target" / "egmap-check"
READER = ROOT / "tools" / "egmap.py"
SAMPLE = ["cargo", "run", "--quiet", "--locked", "-p", "everygraph-map", "--example", "egmap_sample", "--"]
PMTILES_JS = "pmtiles@4.5.0"

spec = importlib.util.spec_from_file_location("egmap", READER)
egmap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(egmap)

JS = r"""
import { open } from "node:fs/promises";
import { createHash } from "node:crypto";
import { PMTiles } from "pmtiles";

class FileSource {
  constructor(path) { this.path = path; }
  getKey() { return this.path; }
  async getBytes(offset, length) {
    const f = await open(this.path);
    const buf = Buffer.alloc(length);
    const { bytesRead } = await f.read(buf, 0, length, offset);
    await f.close();
    const data = buf.buffer.slice(buf.byteOffset, buf.byteOffset + bytesRead);
    return { data };
  }
}

const [path, list] = process.argv.slice(2);
const p = new PMTiles(new FileSource(path));
const header = await p.getHeader();
const metadata = await p.getMetadata();
const tiles = {};
for (const [z, x, y] of JSON.parse(list)) {
  const r = await p.getZxy(z, x, y);
  tiles[`${z}/${x}/${y}`] = r ? createHash("sha256").update(Buffer.from(r.data)).digest("hex") : null;
}
console.log(JSON.stringify({ minZoom: header.minZoom, maxZoom: header.maxZoom,
  numTileEntries: header.numTileEntries, tileType: header.tileType, format: metadata.format, tiles }));
"""


def run(command, cwd=ROOT):
    line = " ".join(str(c) for c in command)
    print("$ " + (line if len(line) < 300 else line[:300] + " …"), flush=True)
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        print(result.stdout + result.stderr)
        raise SystemExit(f"FAILED: exit status {result.returncode}")
    return result.stdout


def dumps(path):
    rust = json.loads(run([*SAMPLE, "dump", path]))
    python = json.loads(run([sys.executable, "-I", READER, "dump", path]))
    return rust, python


def key(t):
    return f"{t['z']}/{t['x']}/{t['y']}"


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    sample = WORK / "sample.egmap"
    run([*SAMPLE, "write", sample])

    rust, python = dumps(sample)
    if "error" in rust or rust != python:
        print("rust:  ", json.dumps(rust)[:2000])
        print("python:", json.dumps(python)[:2000])
        raise SystemExit("FAILED: the Rust and the Python reader read the sample differently")
    if rust["popping"] != 0:
        raise SystemExit(f"FAILED: the sample pyramid pops ({rust['popping']} entities)")
    tiles = {key(t): t for t in rust["tiles"]}
    print(f"ok: both readers read {len(tiles)} tiles, {len(rust['sections'])} arrays, no popping")

    payload_hash = {}
    with open(sample, "rb") as f:
        reader = Reader(MmapSource(f))
        header = reader.header()
        if header["tile_type"].value != 0 or header["max_zoom"] != rust["zooms"][1]:
            raise SystemExit(f"FAILED: the official Python reader sees another header: {header}")
        if reader.metadata().get("format") != "everygraph-egmap":
            raise SystemExit("FAILED: the official Python reader sees other metadata")
        for k, want in tiles.items():
            z, x, y = (int(v) for v in k.split("/"))
            data = reader.get(z, x, y)
            if data is None:
                raise SystemExit(f"FAILED: the official Python reader finds no tile {k}")
            payload = gzip.decompress(data)
            payload_hash[k] = hashlib.sha256(payload).hexdigest()
            if egmap.decode_tile(payload) != want:
                raise SystemExit(f"FAILED: tile {k} from the official Python reader decodes otherwise")
    print(f"ok: the official Python reader (pmtiles 3.8.1) fetched all {len(tiles)} tiles")

    if shutil.which("node") and shutil.which("npm"):
        js = WORK / "js"
        js.mkdir(exist_ok=True)
        if not (js / "node_modules" / "pmtiles").exists():
            run(["npm", "install", "--prefix", js, "--no-audit", "--no-fund", "--silent", PMTILES_JS])
        (js / "check.mjs").write_text(JS)
        listed = json.dumps([[t["z"], t["x"], t["y"]] for t in rust["tiles"]])
        out = json.loads(run(["node", js / "check.mjs", sample.resolve(), listed], cwd=js))
        if out["tiles"] != payload_hash or out["format"] != "everygraph-egmap" or out["tileType"] != 0:
            raise SystemExit("FAILED: the official JavaScript reader reads the sample otherwise")
        print(f"ok: the official JavaScript reader ({PMTILES_JS}) fetched all {len(tiles)} tiles")
    else:
        print("skipped: no node and npm, so the official JavaScript reader was not run")

    data = sample.read_bytes()
    root_len = struct.unpack_from("<Q", data, 16)[0]

    def refused(what, damaged):
        path = WORK / "damaged.egmap"
        path.write_bytes(damaged)
        r, p = dumps(path)
        if "error" not in r or "error" not in p:
            raise SystemExit(f"FAILED: {what} was read (rust: {'error' in r}, python: {'error' in p})")
        print(f"ok: {what}: refused by both")

    counts = bytearray(data)
    counts[80] ^= 1
    refused("header counts that disagree", bytes(counts))
    table = bytearray(data)
    table[127 + root_len] ^= 0x20
    refused("a section table of another magic", bytes(table))
    refused("a cut file", data[:-1])
    refused("a tile entry with a run length of 2", run_of_two(data, root_len))


def varint(b, p):
    v, s = 0, 0
    while True:
        x = b[p]
        p += 1
        v |= (x & 0x7F) << s
        s += 7
        if x < 0x80:
            return v, p


def run_of_two(data, root_len):
    """The sample with the first root entry's run length set to 2, the root re-compressed in
    place (the table moves with it)."""
    raw = bytearray(gzip.decompress(data[127:127 + root_len]))
    n, p = varint(raw, 0)
    for _ in range(n):
        _, p = varint(raw, p)
    assert raw[p] in (0, 1)
    raw[p] = 2 if raw[p] == 1 else 0
    root = gzip.compress(bytes(raw), mtime=0)
    table_at = 127 + root_len
    entries = struct.unpack_from("<I", data, table_at + 12)[0]
    table = data[table_at:table_at + 48 + 64 * entries]
    out = bytearray(data)
    out[127:16384] = bytes(16384 - 127)
    out[127:127 + len(root)] = root
    out[127 + len(root):127 + len(root) + len(table)] = table
    out[16:24] = struct.pack("<Q", len(root))
    return bytes(out)


if __name__ == "__main__":
    main()
