#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Independent reader of the `.egmap` map file, version 1.0.

This reader is written from `docs/spec/egmap.md` (and the public PMTiles v3
specification) alone, without the Rust code. It shows that a third party can implement
a reader from the text. It applies every reading rule of the specification and refuses a
file that breaks one.

Usage:
    python3 -I tools/egmap.py dump PATH          the whole file as one line of JSON
    python3 -I tools/egmap.py tile PATH Z X Y    one tile through the lookup path, or null

A refused file prints {"error": "<message>"} and exits with status 0.

As a module: open_map(path) returns a MapFile with `header`, `table`, `tile(z, x, y)`,
`tiles()`, `metadata()` and `array(name)`; decode_tile(payload) decodes an inflated
payload; tile_id(z, x, y) gives the PMTiles tile id.
"""

import argparse
import json
import math
import os
import struct
import sys
import zlib
from bisect import bisect_right

CAP = 1 << 20
FIRST = 16384
WIDTH = {1: 1, 2: 2, 3: 4, 4: 8, 5: 4}
CODE = {1: "B", 2: "H", 3: "I", 4: "Q", 5: "I"}  # f32 is kept as its bit pattern
HEADER = struct.Struct("<7sB11Q6B4iB2i")
TABLE = struct.Struct("<8sHHIQQQII")
ENTRY = struct.Struct("<32sB3sIQQQ")
NAME_CHARS = set(b"abcdefghijklmnopqrstuvwxyz0123456789._-")
REGION_COLS = "IIIIIQQQIIIBBBB"
NODE_COLS = "IIIIIIQB"
LABEL_COLS = "IIBBB"
KNOWN = {"positions": 5, "adjacency.offsets": 4, "adjacency.targets": 3,
         "matrix.offsets": 4, "matrix.pairs": 3, "matrix.weights": 4}


class EgmapError(Exception):
    pass


def need(cond, msg):
    if not cond:
        raise EgmapError(msg)


def pad8(n):
    return (n + 7) & ~7


def inflate(data, what):
    need(len(data) <= CAP + 4096, f"{what}: compressed size above the cap plus 4 KiB")
    d = zlib.decompressobj(31)
    try:
        out = d.decompress(data, CAP + 1)
    except zlib.error as e:
        raise EgmapError(f"{what}: bad gzip ({e})") from None
    need(len(out) <= CAP, f"{what}: decodes past the cap")
    need(d.eof, f"{what}: gzip stream is truncated")
    need(not d.unused_data, f"{what}: bytes after the gzip member")
    return out


def f32(bits, what):
    need((bits & 0x7F800000) != 0x7F800000, f"{what}: f32 not finite")
    need(bits != 0x80000000, f"{what}: f32 is -0.0")
    return struct.unpack("<f", struct.pack("<I", bits))[0]


def tile_id(z, x, y):
    need(0 <= z <= 31 and 0 <= x < 1 << z and 0 <= y < 1 << z, "tile coordinates out of range")
    n = 1 << z
    d, s = 0, n >> 1
    while s:
        rx, ry = int(x & s > 0), int(y & s > 0)
        d += s * s * ((3 * rx) ^ ry)
        if ry == 0:
            if rx:
                x, y = n - 1 - x, n - 1 - y
            x, y = y, x
        s >>= 1
    return ((1 << 2 * z) - 1) // 3 + d


def varint(buf, pos, what):
    v = shift = 0
    while True:
        need(pos < len(buf), f"{what}: truncated varint")
        b = buf[pos]
        pos += 1
        v |= (b & 0x7F) << shift
        if b < 0x80:
            break
        shift += 7
        need(shift < 64, f"{what}: varint too long")
    need(v < 1 << 64, f"{what}: varint above 64 bits")
    return v, pos


def parse_dir(buf, what):
    """PMTiles v3 directory (§ 2.3) as (id, offset, length, run) tuples."""
    n, p = varint(buf, 0, what)
    need(n * 4 <= len(buf), f"{what}: entry count above what its length holds")
    cols = []
    for _ in range(4):
        col = []
        for _ in range(n):
            v, p = varint(buf, p, what)
            col.append(v)
        cols.append(col)
    need(p == len(buf), f"{what}: bytes after the entries")
    out, tid = [], 0
    for i in range(n):
        delta, run, ln, off = cols[0][i], cols[1][i], cols[2][i], cols[3][i]
        need(i == 0 or delta >= 1, f"{what}: ids do not increase strictly")
        tid += delta
        need(ln >= 1, f"{what}: entry of length 0")
        if off == 0:
            need(i > 0, f"{what}: first entry has no offset")
            off = out[-1][1] + out[-1][2]
        else:
            off -= 1
        out.append((tid, off, ln, run))
    return out


def columns(body, rows, spec, what):
    pos, cols = 0, []
    for code in spec:
        n = rows * struct.calcsize(code)
        need(pos + pad8(n) <= len(body), f"{what}: body shorter than its columns")
        cols.append(struct.unpack_from(f"<{rows}{code}", body, pos))
        need(not any(body[pos + n:pos + pad8(n)]), f"{what}: padding not zero")
        pos += pad8(n)
    need(pos == len(body), f"{what}: length is not the sum of its padded columns")
    return [list(r) for r in zip(*cols)]


def meets(x, y, r, z, tx, ty):
    # § 3.5: closed disc against the closed square, in binary64.
    s = float(1 << z)
    dx = max(tx / s - x, 0.0, x - (tx + 1) / s)
    dy = max(ty / s - y, 0.0, y - (ty + 1) / s)
    return dx * dx + dy * dy <= r * r


def geometry(row, z, tx, ty, what):
    x, y, r = (f32(b, what) for b in row[2:5])
    need(0.0 <= x <= 1.0 and 0.0 <= y <= 1.0, f"{what}: centre outside [0, 1]")
    need(0.0 < r <= 1.0, f"{what}: radius outside (0, 1]")
    need(meets(x, y, r, z, tx, ty), f"{what}: disc does not meet the tile")


def decode_tile(payload):
    """Decode an inflated tile payload and apply the rules of § 4.3."""
    p = payload
    need(len(p) >= 48, "tile: payload shorter than its header")
    magic, major, _minor, z = struct.unpack_from("<4sHHB", p, 0)
    need(magic == b"EGMT", "tile: bad magic")
    need(major == 1, f"tile: later format (major version {major})")
    need(not any(p[9:12]) and not any(p[20:24]), "tile: reserved bytes not zero")
    need(z <= 20, "tile: z above 20")
    x, y = struct.unpack_from("<II", p, 12)
    need(x < 1 << z and y < 1 << z, "tile: x or y not below 2^z")
    omitted = list(struct.unpack_from("<3Q", p, 24))
    blocks, pos, last = {}, 48, -1
    while pos < len(p):
        need(pos + 16 <= len(p), "tile: truncated block header")
        tag, rows, ln = struct.unpack_from("<IIQ", p, pos)
        num = tag & 0x7FFFFFFF
        need(num > last, "tile: block numbers do not increase strictly")
        need(rows >= 1, "tile: block of zero rows")
        need(ln % 8 == 0, "tile: block length not a multiple of 8")
        need(pos + 16 + ln <= len(p), "tile: block body past the payload")
        last, body, pos = num, p[pos + 16:pos + 16 + ln], pos + 16 + ln
        if 1 <= num <= 4:
            need(tag == 0x80000000 | num, f"tile: block {num} without its required bit")
            blocks[num] = (rows, body)
        else:
            need(not tag & 0x80000000, f"tile: unknown required block {num}")
    get = lambda n, spec, what: columns(blocks[n][1], blocks[n][0], spec, what) if n in blocks else []
    regions = get(1, REGION_COLS, "regions")
    nodes = get(2, NODE_COLS, "nodes")
    labels = get(3, LABEL_COLS, "labels")
    strings = []
    if 4 in blocks:
        rows, body = blocks[4]
        head = pad8(4 * rows)
        need(head <= len(body), "strings: body shorter than its ends")
        ends = struct.unpack_from(f"<{rows}I", body, 0)
        need(not any(body[4 * rows:head]), "strings: padding not zero")
        need(all(a <= b for a, b in zip(ends, ends[1:])), "strings: ends decrease")
        need(head + pad8(ends[-1]) == len(body), "strings: last end is not the bytes' length")
        data = body[head:]
        need(not any(data[ends[-1]:]), "strings: padding not zero")
        start = 0
        for e in ends:
            try:
                strings.append(data[start:e].decode("utf-8"))
            except UnicodeDecodeError:
                raise EgmapError("strings: not UTF-8") from None
            start = e
    by_id, prev = {}, 0
    for r in regions:
        rid, parent, nn, internal, volume, comm, rank, partner, kind, depth, colour, zmin = \
            r[0], r[1], r[5], r[6], r[7], r[8], r[9], r[10], r[11], r[12], r[13], r[14]
        what = f"region {rid}"
        need(rid > prev, "regions: ids do not increase strictly")
        prev = rid
        need(parent < rid, f"{what}: parent not below its id")
        if parent == 0:
            need(depth == 1, f"{what}: depth is not 1 under the root")
        else:
            pr = by_id.get(parent)
            need(pr is not None, f"{what}: parent not in the tile")
            need(depth == pr[12] + 1, f"{what}: depth is not its parent's + 1")
            need(colour == pr[13], f"{what}: colour differs from its parent's")
            need(zmin >= pr[14], f"{what}: zmin below its parent's")
        need(kind <= 3 and colour <= 4, f"{what}: kind or colour out of range")
        need(zmin <= z, f"{what}: zmin above the tile's zoom")
        need(nn >= 1 and rank >= 1, f"{what}: nodes or rank is 0")
        need(partner != rid, f"{what}: partner is itself")
        need(2 * internal <= volume, f"{what}: 2 * internal above volume")
        need((comm >= 1) == (kind >= 2), f"{what}: communities does not match kind")
        geometry(r, z, x, y, what)
        by_id[rid] = r
    prev = -1
    for n in nodes:
        what = f"node {n[0]}"
        need(n[0] > prev, "nodes: ids do not increase strictly")
        prev = n[0]
        reg = by_id.get(n[1])
        need(reg is not None, f"{what}: region not in the tile")
        need(reg[14] <= n[7] <= z, f"{what}: zmin below its region's or above z")
        need(n[5] < len(strings), f"{what}: name is not a string of the tile")
        geometry(n, z, x, y, what)
    prev = -1
    for lab in labels:
        ent, text, z_on, z_off, anchor = lab
        need(ent > prev, "labels: entities do not increase strictly")
        prev = ent
        need(ent < len(regions) + len(nodes), "label: entity not in the tile")
        need(text < len(strings), "label: text is not a string of the tile")
        ez = regions[ent][14] if ent < len(regions) else nodes[ent - len(regions)][7]
        need(ez <= z_on <= z < z_off <= 21, "label: zoom interval breaks zmin <= z_on <= z < z_off <= 21")
        need(anchor <= 1, "label: anchor above 1")
    return {"z": z, "x": x, "y": y, "omitted": omitted, "regions": regions, "nodes": nodes,
            "labels": labels, "strings": strings}


class Array:
    def __init__(self, name, element, chunk, count, offset, length):
        self.name, self.element, self.chunk = name, element, chunk
        self.count, self.offset, self.length = count, offset, length
        self.index = None


class MapFile:
    def __init__(self, path):
        self.f = open(path, "rb")
        try:
            self._open()
        except BaseException:
            self.f.close()
            raise

    def close(self):
        self.f.close()

    def _read(self, off, n, what):
        need(off + n <= self.size, f"{what} lies outside the file")
        self.f.seek(off)
        data = self.f.read(n)
        need(len(data) == n, f"{what}: short read")
        return data

    def _open(self):
        size = self.size = os.fstat(self.f.fileno()).st_size
        first = self._read(0, min(FIRST, size), "first bytes")
        need(len(first) >= 127, "file shorter than the PMTiles header")
        v = HEADER.unpack_from(first)
        need(v[0] == b"PMTiles" and v[1] == 3, "not a PMTiles v3 file")
        root_off, root_len, meta_off, meta_len, leaf_off, leaf_len, data_off, data_len = v[2:10]
        addressed, entries, contents, clustered, icomp, tcomp, ttype, minz, maxz = v[10:19]
        need(root_off == 127, "header: root directory offset is not 127")
        need(1 <= root_len and 127 + root_len <= FIRST, "header: root directory must end at or before byte 16384")
        need(meta_len >= 1, "header: metadata length is 0")
        need(addressed == entries == contents, "header: tile counts differ")
        need(clustered == 1, "header: not clustered")
        need(icomp == 2 and tcomp == 2, "header: compression is not gzip")
        need(ttype == 0, "header: tile type is not 0")
        need(minz <= maxz <= 20, "header: bad zooms")
        for what, off, ln in (("root directory", 127, root_len), ("metadata", meta_off, meta_len),
                              ("leaf directories", leaf_off, leaf_len), ("tile data", data_off, data_len)):
            need(off + ln <= size, f"{what} lies outside the file")
        self.header = {"root": (127, root_len), "metadata": (meta_off, meta_len),
                       "leaves": (leaf_off, leaf_len), "data": (data_off, data_len),
                       "tiles": addressed, "min_zoom": minz, "max_zoom": maxz}
        self.root = parse_dir(inflate(first[127:127 + root_len], "root directory"), "root directory")
        leaves = []
        for tid, off, ln, run in self.root:
            need(run in (0, 1), "root directory: run length is not 0 or 1")
            if run:
                need(off + ln <= data_len, "root directory: tile entry outside the tile data")
            else:
                need(off + ln <= leaf_len, "root directory: leaf entry outside the leaf directories")
                leaves.append((off, ln))
        # § 2.3: the leaves fill the leaf directories in id order, each byte once.
        at = 0
        for off, ln in leaves:
            need(off == at, "root directory: leaves share or skip bytes")
            at += ln
        need(at == leaf_len, "root directory: the leaves do not fill the leaf directories")
        self._open_table(first, 127 + root_len)
        spans = [(0, self.table_end, "header, root directory and section table"),
                 (meta_off, meta_len, "metadata"), (leaf_off, leaf_len, "leaf directories"),
                 (data_off, data_len, "tile data")]
        spans += [(a.offset, a.length, f"array {a.name}") for a in self.table["arrays"]]
        spans = sorted(s for s in spans if s[1] > 0)
        for a, b in zip(spans, spans[1:]):
            need(a[0] + a[1] <= b[0], f"{a[2]} and {b[2]} overlap")

    def _open_table(self, first, t0):
        need(t0 + 48 <= len(first), "section table does not fit in the first 16384 bytes")
        magic, major, _minor, n, nodes, regions, edges, k, res = TABLE.unpack_from(first, t0)
        need(magic == b"EGMAPTBL", "not a map: bad section table magic")
        need(major == 1, f"section table: later format (major version {major})")
        need(res == 0, "section table: reserved bytes not zero")
        self.table_end = t0 + 48 + 64 * n
        need(self.table_end <= len(first), "section table does not fit in the first 16384 bytes")
        arrays, names = [], []
        for i in range(n):
            raw, el, res3, chunk, count, off, ln = ENTRY.unpack_from(first, t0 + 48 + 64 * i)
            cut = raw.find(b"\0") if b"\0" in raw else 32
            name = raw[:cut]
            need(cut >= 1 and not any(raw[cut:]) and set(name) <= NAME_CHARS, "section table: bad name")
            need(not names or names[-1] < name, "section table: names do not increase strictly")
            names.append(name)
            need(el in WIDTH and not any(res3), f"section table: bad element or reserved bytes for {name!r}")
            need(1 <= chunk and chunk * WIDTH[el] <= CAP, f"section table: bad chunk for {name!r}")
            need(off + ln <= self.size, f"array {name.decode()} lies outside the file")
            arrays.append(Array(name.decode(), el, chunk, count, off, ln))
        self.table = {"nodes": nodes, "regions": regions, "edges": edges, "summary": k, "arrays": arrays}
        by = {a.name: a for a in arrays}
        count = {"positions": 2 * nodes, "adjacency.offsets": nodes + 1, "matrix.offsets": regions + 1}
        for a in arrays:
            if a.name in KNOWN:
                need(a.element == KNOWN[a.name], f"array {a.name}: wrong element")
                need(a.name not in count or a.count == count[a.name], f"array {a.name}: wrong count")
            need(not (k and (a.name == "positions" or a.name.startswith(("adjacency.", "search.")))),
                 f"summary map holds array {a.name}")
            self._index(a)
        for group in (("adjacency.offsets", "adjacency.targets"), ("matrix.offsets", "matrix.pairs", "matrix.weights")):
            need(len({g in by for g in group}) == 1, f"arrays {', '.join(group)} do not come together")
        if "matrix.pairs" in by:
            need(by["matrix.weights"].count == by["matrix.pairs"].count, "array matrix.weights: wrong count")
        self.arrays_by_name = by

    def _index(self, a):
        k = -(-a.count // a.chunk)
        need(8 * (k + 1) <= a.length, f"array {a.name}: shorter than its chunk index")
        start = a.length - 8 * (k + 1)
        idx = struct.unpack(f"<{k + 1}Q", self._read(a.offset + start, 8 * (k + 1), f"array {a.name}"))
        need(idx[0] == 0 and idx[-1] == start, f"array {a.name}: chunk index does not span the chunks")
        need(all(p < q for p, q in zip(idx, idx[1:])), f"array {a.name}: chunk offsets do not increase strictly")
        a.index = idx

    def array(self, name):
        a = self.arrays_by_name[name]
        out = []
        for i in range(len(a.index) - 1):
            n = min(a.chunk, a.count - i * a.chunk)
            raw = inflate(self._read(a.offset + a.index[i], a.index[i + 1] - a.index[i], f"array {name}"),
                          f"array {name} chunk {i}")
            need(len(raw) == n * WIDTH[a.element], f"array {name} chunk {i}: wrong decoded size")
            vals = struct.unpack(f"<{n}{CODE[a.element]}", raw)
            if a.element == 5:
                for b in vals:
                    f32(b, f"array {name}")
            out.extend(vals)
        return out

    def metadata(self):
        off, ln = self.header["metadata"]
        raw = inflate(self._read(off, ln, "metadata"), "metadata")

        def bad_constant(c):
            raise EgmapError(f"metadata: {c} is not JSON")
        try:
            meta = json.loads(raw.decode("utf-8"), parse_constant=bad_constant)
        except (UnicodeDecodeError, ValueError):
            raise EgmapError("metadata: not UTF-8 JSON") from None
        need(isinstance(meta, dict), "metadata: not a JSON object")
        need(meta.get("format") == "everygraph-egmap", "metadata: format is not everygraph-egmap")
        ver = meta.get("version")
        need(isinstance(ver, str) and ver.partition(".")[0] == "1" and ver.partition(".")[2].isdigit(),
             "metadata: version is not 1.<minor>")
        for key in ("name", "description", "attribution"):
            need(isinstance(meta.get(key), str), f"metadata: {key} is not a string")
        zoom = meta.get("zoom")
        need(isinstance(zoom, dict) and all(
            isinstance(zoom.get(t), (int, float)) and not isinstance(zoom.get(t), bool)
            for t in ("region_px", "open_px", "node_px", "label_off_px")), "metadata: bad zoom thresholds")
        return meta

    def _leaf(self, i):
        tid, off, ln, _ = self.root[i]
        what = f"leaf directory {tid}"
        leaf = parse_dir(inflate(self._read(self.header["leaves"][0] + off, ln, what), what), what)
        nxt = self.root[i + 1][0] if i + 1 < len(self.root) else None
        need(leaf and leaf[0][0] == tid, f"{what}: first id is not the leaf entry's id")
        for e in leaf:
            need(e[3] == 1, f"{what}: run length is not 1")
            need(nxt is None or e[0] < nxt, f"{what}: id reaches the next root entry")
            need(e[1] + e[2] <= self.header["data"][1], f"{what}: entry outside the tile data")
        return leaf

    def _load(self, entry):
        tid, off, ln, _ = entry
        what = f"tile {tid}"
        payload = inflate(self._read(self.header["data"][0] + off, ln, what), what)
        try:
            t = decode_tile(payload)
        except EgmapError as e:
            raise EgmapError(f"{what}: {str(e).removeprefix('tile: ')}") from None
        z = t["z"]
        need(tile_id(z, t["x"], t["y"]) == tid, f"{what}: payload names another tile")
        need(self.header["min_zoom"] <= z <= self.header["max_zoom"], f"{what}: zoom outside the header's zooms")
        need(all(lab[3] <= self.header["max_zoom"] + 1 for lab in t["labels"]), f"{what}: label past max zoom + 1")
        tb = self.table
        need(not (tb["summary"] and t["nodes"]), f"{what}: summary map tile holds nodes")
        for r in t["regions"]:
            need(r[0] < tb["regions"] and r[10] < tb["regions"], f"{what}: region id not below the table's regions")
            need(r[11] != 3 or tb["summary"], f"{what}: region of kind other in a full map")
        need(all(n[0] < tb["nodes"] for n in t["nodes"]), f"{what}: node id not below the table's nodes")
        return t

    def tile(self, z, x, y):
        """§ 2.4 lookup: the last root entry at or below the id, then a leaf if needed."""
        if not (0 <= z <= 20 and 0 <= x < 1 << z and 0 <= y < 1 << z):
            return None
        tid = tile_id(z, x, y)
        i = bisect_right([e[0] for e in self.root], tid) - 1
        if i < 0:
            return None
        entry = self.root[i]
        if entry[3] == 0:
            leaf = self._leaf(i)
            j = bisect_right([e[0] for e in leaf], tid) - 1
            entry = leaf[j]
        return self._load(entry) if entry[0] == tid else None

    def tiles(self):
        """Every tile in tile-id order, checking the tiling of the tile data (§ 2.3)."""
        end, count, zooms = 0, 0, set()
        for i, e in enumerate(self.root):
            for tid, off, ln, _ in (self._leaf(i) if e[3] == 0 else [e]):
                need(off == end, f"tile data: tile {tid} does not start where the one before ends")
                end, count = off + ln, count + 1
                t = self._load((tid, off, ln, 1))
                zooms.add(t["z"])
                yield t
        need(end == self.header["data"][1], "tile data: tiles do not end at the end of the tile data")
        need(count == self.header["tiles"], "header: tile count differs from the directories")
        if zooms:
            need((min(zooms), max(zooms)) == (self.header["min_zoom"], self.header["max_zoom"]),
                 "header: zooms are not the shallowest and deepest zoom with a tile")


def open_map(path):
    return MapFile(path)


def check_arrays(m, vals):
    """The value rules of § 5.3."""
    nodes, regions = m.table["nodes"], m.table["regions"]
    if "positions" in vals:
        need(all(0.0 <= f32(b, "positions") <= 1.0 for b in vals["positions"]), "positions: outside [0, 1]")
    for pre, owners, tgt in (("adjacency", nodes, "targets"), ("matrix", regions, "pairs")):
        if pre + ".offsets" not in vals:
            continue
        off, t = vals[pre + ".offsets"], vals[f"{pre}.{tgt}"]
        need(off[0] == 0 and off[-1] == len(t) and all(a <= b for a, b in zip(off, off[1:])),
             f"{pre}.offsets: not from 0, not decreasing, to the count")
        for u in range(owners):
            s = t[off[u]:off[u + 1]]
            need(all(a < b for a, b in zip(s, s[1:])), f"{pre}.{tgt}: not increasing strictly")
            if pre == "adjacency":
                need(all(v < nodes and v != u for v in s), f"adjacency.targets: bad neighbour of node {u}")
                for v in s:
                    nb = t[off[v]:off[v + 1]]
                    k = bisect_right(nb, u) - 1
                    need(k >= 0 and nb[k] == u, f"adjacency.targets: {u}-{v} not symmetric")
            else:
                need(all(1 <= p >> 16 < (p & 0xFFFF) for p in s), f"matrix.pairs: bad pair in region {u}")
    if "matrix.weights" in vals:
        need(all(w >= 1 for w in vals["matrix.weights"]), "matrix.weights: weight below 1")


def popping(tiles):
    seen = {}
    for t in tiles:
        for kind, rows in (("r", t["regions"]), ("n", t["nodes"])):
            for r in rows:
                seen.setdefault((kind, r[0]), set()).add(t["z"])
    return sum(1 for zs in seen.values() if len(zs) != max(zs) - min(zs) + 1)


def dump(path):
    m = open_map(path)
    try:
        meta = m.metadata()
        tiles = list(m.tiles())
        arrays = {a.name: m.array(a.name) for a in m.table["arrays"]}
        check_arrays(m, arrays)
        tb = m.table
        return {"info": {"nodes": tb["nodes"], "regions": tb["regions"], "edges": tb["edges"],
                         "summary": tb["summary"]},
                "zooms": [m.header["min_zoom"], m.header["max_zoom"]],
                "tile_count": m.header["tiles"], "metadata": meta,
                "sections": [{"name": a.name, "element": a.element, "chunk": a.chunk, "count": a.count}
                             for a in tb["arrays"]],
                "arrays": arrays, "tiles": tiles, "popping": popping(tiles)}
    finally:
        m.close()


def main():
    ap = argparse.ArgumentParser(description="Independent reader of .egmap map files.")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("dump").add_argument("path")
    t = sub.add_parser("tile")
    t.add_argument("path")
    for c in "zxy":
        t.add_argument(c, type=int)
    args = ap.parse_args()
    try:
        if args.cmd == "dump":
            out = dump(args.path)
        else:
            m = open_map(args.path)
            try:
                out = m.tile(args.z, args.x, args.y)
            finally:
                m.close()
    except EgmapError as e:
        out = {"error": str(e)}
    except (OSError, MemoryError, RecursionError) as e:
        out = {"error": f"{type(e).__name__}: {e}"}
    print(json.dumps(out, separators=(",", ":")))


if __name__ == "__main__":
    main()
