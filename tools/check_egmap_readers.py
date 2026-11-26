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
4. Steps 1 to 3 again on a sample of 70,000 tiles, whose directory has leaves.
5. Damaged copies are refused by both the Python and the Rust reader: header counts that
   disagree, a section table of another magic, a cut file, a run length of 2, a header
   max zoom deeper than the tiles, metadata that is not JSON, a tile block numbered 0,
   and a region tree 256 deep.

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


def official(path, tiles):
    """The official readers fetch `tiles` (key -> dumped tile) from `path`, and each tile
    decodes, with the Python reader, to the dumped one. Gives the number fetched by JS,
    or None when node and npm are missing."""
    payload_hash = {}
    with open(path, "rb") as f:
        reader = Reader(MmapSource(f))
        if reader.header()["tile_type"].value != 0:
            raise SystemExit(f"FAILED: the official Python reader sees another header in {path}")
        if reader.metadata().get("format") != "everygraph-egmap":
            raise SystemExit(f"FAILED: the official Python reader sees other metadata in {path}")
        for k, want in tiles.items():
            z, x, y = (int(v) for v in k.split("/"))
            data = reader.get(z, x, y)
            if data is None:
                raise SystemExit(f"FAILED: the official Python reader finds no tile {k} in {path}")
            payload = gzip.decompress(data)
            payload_hash[k] = hashlib.sha256(payload).hexdigest()
            if egmap.decode_tile(payload) != want:
                raise SystemExit(f"FAILED: tile {k} from the official Python reader decodes otherwise")
    if not (shutil.which("node") and shutil.which("npm")):
        return None
    js = WORK / "js"
    js.mkdir(exist_ok=True)
    if not (js / "node_modules" / "pmtiles").exists():
        run(["npm", "install", "--prefix", js, "--no-audit", "--no-fund", "--silent", PMTILES_JS])
    (js / "check.mjs").write_text(JS)
    listed = json.dumps([[int(v) for v in k.split("/")] for k in tiles])
    out = json.loads(run(["node", js / "check.mjs", path.resolve(), listed], cwd=js))
    if out["tiles"] != payload_hash or out["format"] != "everygraph-egmap" or out["tileType"] != 0:
        raise SystemExit(f"FAILED: the official JavaScript reader reads {path} otherwise")
    return len(tiles)


def same(path, what):
    rust, python = dumps(path)
    if "error" in rust or rust != python:
        print("rust:  ", json.dumps(rust)[:2000])
        print("python:", json.dumps(python)[:2000])
        raise SystemExit(f"FAILED: the Rust and the Python reader read {what} differently")
    if rust["popping"] != 0:
        raise SystemExit(f"FAILED: {what} pops ({rust['popping']} entities)")
    return rust


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    sample = WORK / "sample.egmap"
    run([*SAMPLE, "write", sample])
    rust = same(sample, "the sample")
    tiles = {key(t): t for t in rust["tiles"]}
    print(f"ok: both readers read the sample: {len(tiles)} tiles, {len(rust['sections'])} arrays, no popping")
    js = official(sample, tiles)
    print(f"ok: the official Python reader (pmtiles 3.8.1) fetched all {len(tiles)} tiles")
    print(f"ok: the official JavaScript reader ({PMTILES_JS}) fetched all {js} tiles"
          if js else "skipped: no node and npm, so the official JavaScript reader was not run")

    leaves = WORK / "leaves.egmap"
    run([*SAMPLE, "leaves", leaves])
    rust = same(leaves, "the sample with leaf directories")
    if struct.unpack_from("<Q", leaves.read_bytes(), 48)[0] == 0:
        raise SystemExit("FAILED: the sample with leaf directories has none")
    every = rust["tiles"]
    picked = {key(t): t for t in every[:2] + every[4095:4098] + every[-2:]}
    js = official(leaves, picked)
    print(f"ok: both readers read {len(every)} tiles through leaf directories; the official readers "
          f"fetched {len(picked)} of them (Python{', JavaScript' if js else ''})")

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
    refused("a tile entry with a run length of 2", with_root(data, run_of_two))
    deep = bytearray(data)
    deep[101] = 9
    refused("a header max zoom deeper than the tiles", bytes(deep))
    refused("metadata that is not JSON", with_metadata(data, b"not json at all"))
    refused("a tile block number 0", block_zero(data))
    chain = WORK / "chain.egmap"
    run([*SAMPLE, "chain", "256", chain])
    refused("a region tree 256 deep", chain.read_bytes())


def varint(b, p):
    v, s = 0, 0
    while True:
        x = b[p]
        p += 1
        v |= (x & 0x7F) << s
        s += 7
        if x < 0x80:
            return v, p


def put_varint(v):
    out = bytearray()
    while v >= 0x80:
        out.append((v & 0x7F) | 0x80)
        v >>= 7
    out.append(v)
    return bytes(out)


def with_root(data, edit):
    """`data` with its root directory decoded, changed by `edit` (a list of columns: ids,
    runs, lengths, stored offsets), encoded and compressed in place; the section table
    moves with it."""
    root_len = struct.unpack_from("<Q", data, 16)[0]
    raw = gzip.decompress(data[127:127 + root_len])
    n, p = varint(raw, 0)
    cols = []
    for _ in range(4):
        col = []
        for _ in range(n):
            v, p = varint(raw, p)
            col.append(v)
        cols.append(col)
    edit(cols)
    root = gzip.compress(put_varint(n) + b"".join(put_varint(v) for col in cols for v in col), mtime=0)
    table_at = 127 + root_len
    entries = struct.unpack_from("<I", data, table_at + 12)[0]
    table = data[table_at:table_at + 48 + 64 * entries]
    out = bytearray(data)
    out[127:16384] = bytes(16384 - 127)
    out[127:127 + len(root)] = root
    out[127 + len(root):127 + len(root) + len(table)] = table
    out[16:24] = struct.pack("<Q", len(root))
    return bytes(out)


def run_of_two(cols):
    cols[1][0] = 2


def with_metadata(data, text):
    """`data` with other metadata: the writer puts the metadata last."""
    at, length = struct.unpack_from("<QQ", data, 24)
    assert at + length == len(data)
    packed = gzip.compress(text, mtime=0)
    out = bytearray(data[:at] + packed)
    out[32:40] = struct.pack("<Q", len(packed))
    return bytes(out)


def block_zero(data):
    """`data` with its first tile's first block numbered 0 (bit 31 kept); the tile data,
    with the changed tile, moves to the end of the file."""
    tiles_at, tiles_len = struct.unpack_from("<QQ", data, 56)
    root_len = struct.unpack_from("<Q", data, 16)[0]
    raw = gzip.decompress(data[127:127 + root_len])
    n, p = varint(raw, 0)
    for _ in range(2 * n):
        _, p = varint(raw, p)
    first, _ = varint(raw, p)
    payload = bytearray(gzip.decompress(data[tiles_at:tiles_at + first]))
    payload[48:52] = struct.pack("<I", 0x80000000)
    packed = gzip.compress(bytes(payload), mtime=0)
    moved = packed + data[tiles_at + first:tiles_at + tiles_len]

    def edit(cols):
        cols[2][0] = len(packed)

    out = bytearray(with_root(data, edit) + moved)
    out[56:72] = struct.pack("<QQ", len(data), len(moved))
    return bytes(out)


if __name__ == "__main__":
    main()
