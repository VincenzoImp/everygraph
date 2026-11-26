# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Numbers, sizes and durations: read as `docs/api-sketch.md` § 2.3 says, printed as
`docs/ux-contract.md` § 2.6 says."""

from __future__ import annotations

import re

_UNITS = {
    "b": 1,
    "kb": 10**3,
    "mb": 10**6,
    "gb": 10**9,
    "tb": 10**12,
    "kib": 2**10,
    "mib": 2**20,
    "gib": 2**30,
    "tib": 2**40,
}
_SIZE = re.compile(r"\s*(\d+(?:\.\d+)?)\s*([a-zA-Z]*)\s*")


def parse_size(value: str | int, name: str = "memory") -> int:
    """Bytes of a size: an `int` is bytes; text is a number and a unit of `kB MB GB TB`
    (powers of 10) or `KiB MiB GiB TiB` (powers of 2), in any case."""
    if isinstance(value, bool):
        raise TypeError(f"{name} must be a size such as \"8GB\" or a number of bytes")
    if isinstance(value, int):
        if value < 1:
            raise ValueError(f"{name}={value}: a size is at least 1 byte")
        return value
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a size such as \"8GB\" or a number of bytes")
    match = _SIZE.fullmatch(value)
    unit = _UNITS.get(match.group(2).lower() or "b") if match else None
    if match is None or unit is None:
        raise ValueError(f"{name}={value!r} is not a size, such as \"8GB\"")
    size = int(float(match.group(1)) * unit)
    if size < 1:
        raise ValueError(f"{name}={value!r}: a size is at least 1 byte")
    return size


def count(n: int) -> str:
    """An exact count, with thousands separators."""
    return f"{n:,}"


def counted(n: int, one: str, many: str) -> str:
    """`1 row`, `2 rows`."""
    return f"{n:,} {one if n == 1 else many}"


def size(n: int) -> str:
    """An exact size or a budget: one decimal below 100 of its unit."""
    if n < 1000:
        return f"{n} B"
    for unit, scale in (("kB", 10**3), ("MB", 10**6), ("GB", 10**9), ("TB", 10**12)):
        value = n / scale
        if value < 1000 or unit == "TB":
            return f"{value:.1f} {unit}" if value < 100 else f"{value:.0f} {unit}"
    raise AssertionError("unreachable")


def typed(n: int) -> str:
    """A size as it is typed: the smallest whole number of a unit that holds `n`
    (`6GB`), rounded up so that it is enough."""
    for unit, scale in (("TB", 10**12), ("GB", 10**9), ("MB", 10**6), ("kB", 10**3)):
        if n >= scale:
            return f"{-(-n // scale)}{unit}"
    return str(n)


def duration(seconds: float) -> str:
    """`1.3 ms`, `38 s`, `1 min 41 s`, `2 h 5 min`."""
    if seconds < 1:
        ms = seconds * 1000
        return f"{ms:.1f} ms" if ms < 100 else f"{ms:.0f} ms"
    s = int(round(seconds))
    if s < 60:
        return f"{s} s"
    if s < 3600:
        m, s = divmod(s, 60)
        return f"{m} min {s} s" if s else f"{m} min"
    h, rest = divmod(s, 3600)
    m = rest // 60
    return f"{h} h {m} min" if m else f"{h} h"
