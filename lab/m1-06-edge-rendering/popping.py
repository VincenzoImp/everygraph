#!/usr/bin/env python3
"""Zoom consistency of the lab tiles: does an entity shown at one zoom vanish at a deeper one?

Reads every tile of a lab map (PMTiles v3, payload GMT1 of lab/e-map/gmap/src/tiles.rs) and records, per node and
per region (tree node), the zoom levels whose tiles carry it. A node sits in the tile that holds its centre; a
region counts as shown at a zoom if any tile carries it, as a bubble or as context. Counted:
  - vanish: shown at z, not shown at z+1 (z < zmax)
  - popping: shown at z1, not at z2, shown again at z3 (z1 < z2 < z3): the case a monotone rule removes
Load-independent counts.

    popping.py MAP.pmtiles OUT.json
"""
import gzip
import json
import mmap
import struct
import sys

import numpy as np


def varint(b, p):
    r, s = 0, 0
    while True:
        x = b[p]
        p += 1
        r |= (x & 0x7F) << s
        s += 7
        if x < 0x80:
            return r, p


def directory(raw):
    b = gzip.decompress(raw)
    n, p = varint(b, 0)
    ids, runs, lens, offs = [], [], [], []
    last = 0
    for _ in range(n):
        d, p = varint(b, p)
        last += d
        ids.append(last)
    for _ in range(n):
        r, p = varint(b, p)
        runs.append(r)
    for _ in range(n):
        ln, p = varint(b, p)
        lens.append(ln)
    for i in range(n):
        o, p = varint(b, p)
        offs.append(offs[i - 1] + lens[i - 1] if (o == 0 and i > 0) else o - 1)
    return list(zip(ids, runs, lens, offs))


path, out = sys.argv[1], sys.argv[2]
f = open(path, "rb")
mm = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)
h = mm[:127]
root_off, root_len, meta_off, meta_len, leaf_off, leaf_len, data_off, data_len = struct.unpack_from("<8Q", h, 8)
zmax = h[101]
meta = json.loads(gzip.decompress(mm[meta_off:meta_off + meta_len]))
n = int(meta["n"])

entries = []
for e in directory(mm[root_off:root_off + root_len]):
    if e[1] == 0:
        entries += directory(mm[leaf_off + e[3]:leaf_off + e[3] + e[2]])
    else:
        entries.append(e)

BUB = np.dtype([("id", "<u4"), ("rest", "V44")])
NODE = np.dtype([("id", "<u4"), ("rest", "V32")])
node_mask = np.zeros(n, dtype=np.uint16)
reg = {}
tiles = 0
for tid, run, ln, off in entries:
    t = gzip.decompress(mm[data_off + off:data_off + off + ln])
    assert t[:4] == b"GMT1", t[:4]
    z = t[4]
    nctx, nbub, nnode = struct.unpack_from("<3I", t, 16)
    b = np.frombuffer(t, dtype=BUB, count=nctx + nbub, offset=72)["id"]
    nd = np.frombuffer(t, dtype=NODE, count=nnode, offset=72 + 48 * (nctx + nbub))["id"]
    node_mask[nd] |= np.uint16(1 << z)
    for i in b.tolist():
        reg[i] = reg.get(i, 0) | (1 << z)
    tiles += 1


def count(masks):
    masks = np.asarray(masks, dtype=np.int64)
    shown = masks != 0
    m = masks[shown]
    zs = np.arange(zmax + 1)
    bits = (m[:, None] >> zs) & 1
    first = bits.argmax(1)
    last = zmax - bits[:, ::-1].argmax(1)
    vanish = ((bits[:, :-1] == 1) & (bits[:, 1:] == 0)).sum()
    span = (zs >= first[:, None]) & (zs <= last[:, None])
    gaps = (span & (bits == 0)).any(1)
    return {"shown_at_some_zoom": int(shown.sum()), "vanish_events": int(vanish),
            "entities_that_vanish_before_zmax": int((last < zmax).sum()),
            "entities_with_popping": int(gaps.sum()), "popping_gap_levels": int((span & (bits == 0)).sum()),
            "first_zoom_histogram": np.bincount(first, minlength=zmax + 1).tolist(),
            "vanish_events_by_zoom": ((bits[:, :-1] == 1) & (bits[:, 1:] == 0)).sum(0).tolist()}


res = {"map": path, "zmax": int(zmax), "tiles": tiles, "nodes": count(node_mask), "regions": count(list(reg.values()))}
json.dump(res, open(out, "w"), indent=1)
print(json.dumps(res, indent=1))
