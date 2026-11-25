#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""An independent reader of the .eg container, written from docs/spec/eg-container.md
alone, to show that the specification is enough (#28). Standard library only: BLAKE3
is not in hashlib, so a plain implementation of it follows the reader (the `blake3`
package is used instead when installed).

    eg_container.py list FILE [--json]   the sections of the newest usable commit
    eg_container.py verify FILE          check every section's hash and padding
"""
import json
import struct
import sys

MAGIC, TOC_MAGIC, NONE, FOREIGN = b"\x89EGRAPH\n", b"EGTOC\0\0\0", 0xFFFFFFFF, []


def pad_end(offset, length):  # § 1
    return -(-(offset + length + 16) // 4096) * 4096


def header(block):  # § 2: (generation, toc offset, toc length, toc hash), or None
    if len(block) < 128 or block[:8] != MAGIC:
        return None
    order, major, minor = struct.unpack_from("<IHH", block, 8)
    if order != 0x0A0B0C0D or major != 1:
        FOREIGN.append(f"not readable here: byte order {order:#x}, version {major}.{minor}")
        return None
    if blake3(block[:96]) != block[96:128]:
        return None  # torn
    generation, required, _optional, toc_offset, toc_len = struct.unpack_from("<5Q", block, 24)
    if required:
        raise SystemExit(f"needs unknown features {required:#x}")
    return generation, toc_offset, toc_len, block[64:96]


def entries(toc, toc_offset):  # § 4: [(name, kind, codec, offset, length, hash, provenance)]
    assert toc[:8] == TOC_MAGIC, "TOC magic"
    n, size, names_len = struct.unpack_from("<IIQ", toc, 8)
    assert size >= 72 and size % 8 == 0 and len(toc) == 32 + n * size + names_len, "TOC length"
    names, out = toc[32 + n * size:], []
    for i in range(n):
        kind, codec, _, offset, length = struct.unpack_from("<IHHQQ", toc, 32 + i * size)
        name_at, name_len, prov = struct.unpack_from("<III", toc, 32 + i * size + 56)
        assert kind and 1 <= name_len <= 1024 and name_at + name_len <= len(names), "entry"
        assert offset % 4096 == 0 and offset >= 8192 and pad_end(offset, length) <= toc_offset
        h = toc[32 + i * size + 24:32 + i * size + 56]
        out.append([names[name_at:name_at + name_len].decode(), kind, codec, offset, length, h, prov])
    assert all(a[0].encode() < b[0].encode() for a, b in zip(out, out[1:])), "names unsorted"
    spans = sorted((e[3], e[4]) for e in out)
    assert all(pad_end(*a) <= b[0] for a, b in zip(spans, spans[1:])), "sections overlap"
    for e in out:
        if e[6] != NONE:
            assert e[6] < n and out[e[6]][1] == 1 and out[e[6]] is not e, "provenance"
            e[6] = out[e[6]][0]
        else:
            e[6] = None
    return out


def open_container(f):  # § 7: two reads
    f.seek(0)
    head = f.read(4096 + 128)
    blocks = [h for h in (header(head[:128]), header(head[4096:])) if h]
    if not blocks:
        raise SystemExit(FOREIGN[0] if FOREIGN else "no valid header block")
    newest = max(blocks)[0]
    for generation, toc_offset, toc_len, toc_hash in sorted(blocks, reverse=True):
        if toc_offset % 4096 or toc_offset < 8192 or not 32 <= toc_len <= 16 << 20:
            continue
        f.seek(toc_offset)
        toc = f.read(toc_len)
        if len(toc) == toc_len and blake3(toc) == toc_hash:
            return generation, newest, entries(toc, toc_offset)
    raise SystemExit("no usable table of contents")


def verify(f, entry):  # § 7: the hash of the stored bytes, and zero padding
    _, _, _, offset, length, h, _ = entry
    f.seek(offset)
    data = f.read(length)
    padding = f.read(pad_end(offset, length) - offset - length)
    return blake3(data) == h and padding == bytes(len(padding))


# BLAKE3, hash mode, 32-byte output (https://github.com/BLAKE3-team/BLAKE3-specs).
IV = (0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19)
PERM = (2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8)
START, END, PARENT, ROOT, M = 1, 2, 4, 8, 0xFFFFFFFF


def compress(cv, m, counter, length, flags):
    v = list(cv) + list(IV[:4]) + [counter & M, counter >> 32, length, flags]

    def g(a, b, c, d, x, y):
        v[a] = (v[a] + v[b] + x) & M; v[d] = ((v[d] ^ v[a]) >> 16 | (v[d] ^ v[a]) << 16) & M
        v[c] = (v[c] + v[d]) & M; v[b] = ((v[b] ^ v[c]) >> 12 | (v[b] ^ v[c]) << 20) & M
        v[a] = (v[a] + v[b] + y) & M; v[d] = ((v[d] ^ v[a]) >> 8 | (v[d] ^ v[a]) << 24) & M
        v[c] = (v[c] + v[d]) & M; v[b] = ((v[b] ^ v[c]) >> 7 | (v[b] ^ v[c]) << 25) & M

    for r in range(7):
        g(0, 4, 8, 12, m[0], m[1]); g(1, 5, 9, 13, m[2], m[3]); g(2, 6, 10, 14, m[4], m[5])
        g(3, 7, 11, 15, m[6], m[7]); g(0, 5, 10, 15, m[8], m[9]); g(1, 6, 11, 12, m[10], m[11])
        g(2, 7, 8, 13, m[12], m[13]); g(3, 4, 9, 14, m[14], m[15])
        m = [m[i] for i in PERM]
    return [v[i] ^ v[i + 8] for i in range(8)]


def blake3_pure(data):
    chunks = [data[i:i + 1024] for i in range(0, len(data), 1024)] or [b""]
    stack = []
    for ci, chunk in enumerate(chunks):
        cv, blocks = IV, [chunk[i:i + 64] for i in range(0, len(chunk), 64)] or [b""]
        for bi, block in enumerate(blocks):
            flags = (START if bi == 0 else 0) | (END if bi == len(blocks) - 1 else 0)
            node = (cv, struct.unpack("<16I", block.ljust(64, b"\0")), ci, len(block), flags)
            cv = compress(*node)
        if ci < len(chunks) - 1:  # a finished chunk that is not the last: merge
            t = ci + 1
            while t % 2 == 0:
                cv = compress(IV, stack.pop() + cv, 0, 64, PARENT)
                t //= 2
            stack.append(cv)
    while stack:
        node = (IV, stack.pop() + compress(*node), 0, 64, PARENT)
    cv, m, counter, length, flags = node
    return struct.pack("<8I", *compress(cv, m, counter, length, flags | ROOT))


try:
    from blake3 import blake3 as _fast

    def blake3(data):
        return _fast(data).digest()
except ImportError:
    blake3 = blake3_pure


def main(argv):
    if len(argv) < 3 or argv[1] not in ("list", "verify"):
        raise SystemExit(__doc__)
    with open(argv[2], "rb") as f:
        generation, newest, found = open_container(f)
        if newest != generation:
            print(f"warning: commit {newest} is unusable; reading commit {generation}", file=sys.stderr)
        if argv[1] == "verify":
            bad = [e[0] for e in found if not verify(f, e)]
            print(f"{len(found) - len(bad)} of {len(found)} sections verified" + "".join(f"\nDAMAGED {n}" for n in bad))
            return 1 if bad else 0
    keys = ("name", "kind", "codec", "offset", "length", "hash", "provenance")
    rows = [dict(zip(keys, e[:5] + [e[5].hex(), e[6]])) for e in found]
    if "--json" in argv:
        print(json.dumps({"generation": generation, "sections": rows}))
    else:
        print(f"commit {generation}, {len(rows)} sections")
        for r in rows:
            print(f"{r['offset']:>12} {r['length']:>12}  kind {r['kind']:#x} codec {r['codec']}  {r['hash'][:16]}  {r['name']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
