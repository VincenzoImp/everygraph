#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyarrow==26.0.1", "polars==2.2.0", "numpy==2.5.5"]
# ///
"""The column check: pyarrow and Polars read the columns Rust wrote, without Everygraph.

`everygraph-io`'s example `columns_sample` writes a `.eg` file of property columns
(docs/spec/eg-columns.md) of every type, with nulls, NaNs, -0.0, infinities, extreme
integers and several batches, and prints what it gave the writer. For every column
this script, with the standard-library container reader `tools/eg_container.py` to find
the sections and its own reader of the layout and statistics records written from the
specification:

- opens the column section with `pyarrow.ipc.open_file` on a slice of a memory map of
  the file, and requires the values to be those given to the writer (floats stored
  canonically: one quiet NaN, no -0.0; nulls where they were);
- requires every buffer pyarrow returns to lie inside the map, at a multiple of 4096,
  where the layout section says (zero-copy: pyarrow did not copy it);
- recomputes the statistics from the values pyarrow read (sums exactly, with
  `fractions.Fraction`, rounded once) and requires the statistics section to hold them;
- reads the same bytes with Polars and requires the same values (decimal256, which
  Polars does not read, excepted and reported).

With `--big` it runs instead the check of docs/research/sota-ingest-query.md C4: a
float32 column of 2^27 rows in two batches and a dictionary-encoded uint8 column,
written twice (RAYON_NUM_THREADS=1 and 32) to identical bytes, then read as above.
Run it under a memory limit (about 3 GB):

    uv run --script tools/check_columns_python.py [--big]

Exit status 0 if the check passes, 1 otherwise.
"""

import hashlib
import importlib.util
import io
import json
import math
import os
import pathlib
import re
import shutil
import struct
import subprocess
import sys
import decimal
from fractions import Fraction

import numpy as np
import polars as pl
import pyarrow as pa

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORK = ROOT / "target" / "columns-python"
SAMPLE = ["cargo", "run", "--quiet", "--locked", "-p", "everygraph-io", "--example", "columns_sample", "--"]

spec = importlib.util.spec_from_file_location("eg_container", ROOT / "tools" / "eg_container.py")
eg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(eg)

# Decimal arithmetic exact to decimal256's 76 digits and more.
decimal.getcontext().prec = 200

QNAN64, QNAN32 = 0x7FF8000000000000, 0x7FC00000
WIDTH = {"Int8": 1, "Int16": 2, "Int32": 4, "Int64": 8, "UInt8": 1, "UInt16": 2, "UInt32": 4, "UInt64": 8,
         "Float32": 4, "Float64": 8, "Decimal128": 16, "Decimal256": 32, "Timestamp": 8}


def fail(what):
    raise SystemExit(f"FAILED: {what}")


def need(condition, what):
    if not condition:
        fail(what)


def run(command, env=None):
    print("$ " + " ".join(str(c) for c in command), flush=True)
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, env=env)
    if result.returncode != 0:
        print(result.stdout + result.stderr)
        fail(f"exit status {result.returncode}")
    return result.stdout


def parse_layout(b):
    """docs/spec/eg-columns.md § 4."""
    magic, version, flags, rows, batch_rows, batches, column_len = struct.unpack_from("<8sIIQQQQ", b, 0)
    need(magic == b"EGCOLLAY" and version == 1 and flags == 0, "layout header")
    code, nullable, _, p1, p2, name_len, values, data = struct.unpack_from("<BBHIiIQQ", b, 48)
    at = 80 + -(-name_len // 8) * 8
    out = {"rows": rows, "batch_rows": batch_rows, "column_len": column_len, "code": code,
           "nullable": nullable, "p1": p1, "p2": p2, "name": b[80:80 + name_len].decode(), "batches": []}
    if code == 8:
        out["dictionary"] = {"values": values, "offsets": struct.unpack_from("<2Q", b, at),
                             "data": struct.unpack_from("<2Q", b, at + 16)}
        at += 32
    for i in range(batches):
        nulls, v_off, v_len, x_off, x_len = struct.unpack_from("<5Q", b, at + 40 * i)
        out["batches"].append({"nulls": nulls, "validity": (v_off, v_len), "values": (x_off, x_len)})
    need(at + 40 * batches == len(b), "layout length")
    return out


def parse_stats(b):
    """docs/spec/eg-columns.md § 5."""
    need(len(b) == 112 and b[:8] == b"EGCOLSTA", "statistics record")
    version, flags, count, nulls, nans, total = struct.unpack_from("<IIQQQQ", b, 8)
    need(version == 1 and flags in (0, 1), "statistics version and flags")
    return {"count": count, "null_count": nulls, "nan_count": nans, "sum": total,
            "range": (b[48:80], b[80:112]) if flags else None}


def kind_of(t):
    return t.split("(")[0].split(" ")[0]


def canonical(bits, width):
    if width == 8:
        x = struct.unpack("<d", struct.pack("<Q", bits))[0]
        return QNAN64 if math.isnan(x) else (0 if x == 0 else bits)
    x = struct.unpack("<f", struct.pack("<I", bits))[0]
    return QNAN32 if math.isnan(x) else (0 if x == 0 else bits)


def stored_values(column, arrow):
    """What the sample gave the writer, as pyarrow should read it back."""
    t = kind_of(column["type"])
    values = column["values"]
    if t in ("Float32", "Float64"):
        return [None if v is None else canonical(int(v, 16), WIDTH[t]) for v in values]
    if t == "Decimal256":
        return [None if v is None else v[0] * 2**128 + v[1] for v in values]
    if t == "FixedSizeBinary":
        return [None if v is None else bytes.fromhex(v) for v in values]
    return values


def read_values(column, arrow, mapped_buffers):
    """The values of the pyarrow array `arrow`, comparable with `stored_values`."""
    t = kind_of(column["type"])
    if t in ("Float32", "Float64"):
        fmt = "<Q" if t == "Float64" else "<I"
        width = WIDTH[t]
        out = []
        for chunk in arrow.chunks:
            raw = chunk.buffers()[1].to_pybytes()
            for i, valid in enumerate(chunk.is_valid().to_pylist()):
                out.append(struct.unpack_from(fmt, raw, (chunk.offset + i) * width)[0] if valid else None)
        return out
    if t == "Timestamp":
        return arrow.cast(pa.int64()).to_pylist()
    if t in ("Decimal128", "Decimal256"):
        scale = arrow.type.scale
        return [None if v is None else int(v.scaleb(scale)) for v in arrow.to_pylist()]
    return arrow.to_pylist()


def expected_stats(column, arrow):
    """The statistics of § 5, computed here from what pyarrow read."""
    t = kind_of(column["type"])
    values = [v for v in read_values(column, arrow, None) if v is not None]
    nulls = arrow.null_count
    width = WIDTH[t]
    if t in ("Float32", "Float64"):
        unpack = (lambda b: struct.unpack("<d", struct.pack("<Q", b))[0]) if t == "Float64" else \
            (lambda b: struct.unpack("<f", struct.pack("<I", b))[0])
        xs = [unpack(v) for v in values]
        nans = sum(math.isnan(x) for x in xs)
        xs = [x for x in xs if not math.isnan(x)]
        pos, neg = math.inf in xs, -math.inf in xs
        finite = [Fraction(x) for x in xs if math.isfinite(x)]
        if pos and neg:
            total = math.nan
        elif pos or neg:
            total = math.inf if pos else -math.inf
        else:
            total = round_once(sum(finite, Fraction(0)))
        pack = (lambda x: struct.pack("<d", x)) if t == "Float64" else (lambda x: struct.pack("<f", x))
        rng = (pack(min(xs)), pack(max(xs))) if xs else None
    else:
        nans = 0
        scale = arrow.type.scale if t.startswith("Decimal") else 0
        total = round_once(sum((Fraction(v) / Fraction(10) ** scale for v in values), Fraction(0)))
        signed = not t.startswith("UInt")
        pack = lambda v: v.to_bytes(width, "little", signed=signed)
        rng = (pack(min(values)), pack(max(values))) if values else None
    bits = QNAN64 if math.isnan(total) else struct.unpack("<Q", struct.pack("<d", total))[0]
    pad = lambda b: b + bytes(32 - len(b))
    return {"count": len(values), "null_count": nulls, "nan_count": nans, "sum": bits,
            "range": (pad(rng[0]), pad(rng[1])) if rng else None}


def round_once(q):
    """The exact value `q` rounded to the nearest float, ties to even; never -0.0."""
    try:
        x = q.numerator / q.denominator
    except OverflowError:
        x = math.inf if q > 0 else -math.inf
    return x if x != 0 else 0.0


def check_buffers(arrow_file, layout, base, offset):
    """Every buffer pyarrow returns lies in the map where the layout says, at 4 KiB."""
    need(arrow_file.num_record_batches == len(layout["batches"]), "batch count")
    for i, want in enumerate(layout["batches"]):
        array = arrow_file.get_batch(i).column(0)
        rows = min(layout["batch_rows"], layout["rows"] - i * layout["batch_rows"])
        need(len(array) == rows and array.null_count == want["nulls"], f"batch {i} rows and nulls")
        if isinstance(array, pa.DictionaryArray):
            dictionary = array.dictionary.buffers()
            d = layout["dictionary"]
            for buf, (at, length) in zip(dictionary[1:], (d["offsets"], d["data"])):
                if length:
                    need(buf.address - base == offset + at and (buf.address - base) % 4096 == 0,
                         "a dictionary buffer is not where the layout says")
            array = array.indices
        validity, values = array.buffers()
        for buf, (at, length), what in ((validity, want["validity"], "validity"), (values, want["values"], "values")):
            if length == 0:
                continue
            address = buf.address - base
            need(address == offset + at, f"batch {i}: the {what} buffer is at {address}, the layout says {offset + at}")
            need(address % 4096 == 0, f"batch {i}: the {what} buffer is not at a multiple of 4096")
            need(buf.size >= length, f"batch {i}: the {what} buffer is short")


def column_files(path):
    with open(path, "rb") as f:
        _, _, _, entries = eg.open_container(f)
    return {e[0]: (e[1], e[3], e[4]) for e in entries}


def nan_as_text(values):
    return [None if v is None else ("nan" if isinstance(v, float) and math.isnan(v) else v) for v in values]


def polars_values(data):
    return nan_as_text(pl.read_ipc(io.BytesIO(data)).to_series(0).to_list())


def small():
    path = WORK / "sample.eg"
    columns = json.loads(run([*SAMPLE, "write", path]))["columns"]
    sections = column_files(path)
    data = path.read_bytes()
    mapped = pa.memory_map(str(path))
    whole = mapped.read_buffer()
    base = whole.address
    polars_skipped = []
    for column in columns:
        names = {r: f"{column['scope']}/{r}/{column['name']}" for r in ("arrow", "layout", "stats")}
        kind, offset, length = sections[names["arrow"]]
        need(kind == 7, "column kind")
        lk, lo, ll = sections[names["layout"]]
        need(lk == 8, "layout kind")
        layout = parse_layout(data[lo:lo + ll])
        need(layout["column_len"] == length and layout["name"] == column["name"], "layout against column")
        arrow_file = pa.ipc.open_file(whole.slice(offset, length))
        table = arrow_file.read_all()
        need(table.schema.names == [column["name"]], "field name")
        need(table.schema.field(0).nullable == column["nullable"], "nullability")
        arrow = table.column(0)
        check_buffers(arrow_file, layout, base, offset)
        got = read_values(column, arrow, None)
        want = stored_values(column, arrow)
        need(got == want, f"{names['arrow']}: pyarrow read {got}, the writer was given {want}")
        t = kind_of(column["type"])
        has_stats = names["stats"] in sections
        need(has_stats == (t in WIDTH), f"{names['arrow']}: statistics present for numeric columns only")
        if has_stats:
            sk, so, sl = sections[names["stats"]]
            need(sk == 9, "statistics kind")
            stats, expected = parse_stats(data[so:so + sl]), expected_stats(column, arrow)
            need(stats == expected, f"{names['stats']}: {stats} != {expected}")
        if t == "Decimal256":  # Polars 2.2 does not read decimal256 (it panics)
            polars_skipped.append(names["arrow"])
            continue
        from_polars = polars_values(data[offset:offset + length])
        from_arrow = nan_as_text(arrow.to_pylist())
        need(from_polars == from_arrow, f"Polars read {names['arrow']} as {from_polars}, pyarrow as {from_arrow}")
        print(f"ok: {names['arrow']}: {len(arrow)} rows, {len(layout['batches'])} batches"
              f"{', statistics' if has_stats else ''}; pyarrow zero-copy at 4 KiB; Polars agrees")
    print("not read with Polars, which has no decimal256: " + (", ".join(polars_skipped) or "nothing"))
    del whole
    mapped.close()

    # The golden test pins these sections' hashes; the bytes it pins are the ones read above.
    test = (ROOT / "crates" / "everygraph-format" / "tests" / "columns.rs").read_text()
    pinned = re.findall(r'\(\s*"[^"/]+/(arrow|layout|stats)/(\w+)",\s*"([0-9a-f]{64})",?\s*\)', test)
    need(len(pinned) == 8, f"found {len(pinned)} golden hashes in tests/columns.rs, not 8")
    for role, name, digest in pinned:
        _, offset, length = sections[f"golden/{role}/{name}"]
        need(eg.blake3(data[offset:offset + length]).hex() == digest, f"golden/{role}/{name} is not the golden test's")
    print(f"ok: the {len(pinned)} sections of the golden test are among those read")


def big():
    files = []
    for threads in ("1", "32"):
        path = WORK / f"big-{threads}.eg"
        run([*SAMPLE, "write", path, "--big"], env={**os.environ, "RAYON_NUM_THREADS": threads})
        files.append(path)
    digests = [hashlib.sha256(p.read_bytes()).hexdigest() for p in files]
    need(digests[0] == digests[1], "the files written at 1 and 32 threads differ")
    print(f"ok: identical bytes at 1 and 32 threads (sha256 {digests[0][:16]}…)")
    path = files[0]
    sections = column_files(path)
    data = path.read_bytes()
    mapped = pa.memory_map(str(path))
    whole = mapped.read_buffer()
    rows = 1 << 27
    expected = {"f32": np.arange(rows, dtype=np.int64).astype(np.float32),
                "kind": np.array(["a", "bb", "ccc"])[np.arange(1 << 20) % 3]}
    for name, want in expected.items():
        kind, offset, length = sections[f"big/arrow/{name}"]
        _, lo, ll = sections[f"big/layout/{name}"]
        layout = parse_layout(data[lo:lo + ll])
        arrow_file = pa.ipc.open_file(whole.slice(offset, length))
        check_buffers(arrow_file, layout, whole.address, offset)
        column = arrow_file.read_all().column(0)
        if name == "f32":
            need(len(layout["batches"]) == 2, "the 2^27-row column is not in two batches")
            got = np.concatenate([c.to_numpy(zero_copy_only=True) for c in column.chunks])
        else:
            got = np.array(column.to_pylist())
        need(np.array_equal(got, want), f"big/{name}: pyarrow values differ")
        frame = pl.read_ipc(io.BytesIO(data[offset:offset + length]))
        polars = frame.to_series(0).to_numpy() if name == "f32" else np.array(frame.to_series(0).cast(pl.String).to_list())
        need(np.array_equal(polars, want), f"big/{name}: Polars values differ")
        print(f"ok: big/{name}: {len(column)} rows in {len(layout['batches'])} batches, buffers inside the map "
              f"at multiples of 4096, round trip through pyarrow and Polars")
    del whole
    mapped.close()


def main(argv):
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    big() if "--big" in argv else small()
    shutil.rmtree(WORK)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
