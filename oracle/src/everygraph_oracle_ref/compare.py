# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Comparators, with the tolerances of oracle/tolerances.json (shared with the Rust crate)."""

from __future__ import annotations

import json
import math
from pathlib import Path

TOLERANCES_FILE = Path(__file__).resolve().parents[2] / "tolerances.json"
_TOL = {k: v for k, v in json.loads(TOLERANCES_FILE.read_text()).items() if not k.startswith("_")}


def tolerance(name: str) -> dict:
    return _TOL.get(name, {"abs": 0.0, "abs_over_n": 0.0, "rel": 0.0})


def close(got: float, want: float, n: int, tol: dict) -> bool:
    if not (math.isfinite(got) and math.isfinite(want)):
        return got == want
    per_n = tol["abs_over_n"] / n if n else 0.0
    return abs(got - want) <= tol["abs"] + per_n + tol["rel"] * abs(want)


def canonical_labels(labels: list[int]) -> list[int]:
    first: dict[int, int] = {}
    return [first.setdefault(lab, v) for v, lab in enumerate(labels)]


def compare(kernel: str, n: int, got: dict, want: dict) -> str | None:
    """None if equal under the oracle's rules, else a short description."""
    (shape_g, a), = got.items()
    (shape_w, b), = want.items()
    if shape_g != shape_w:
        return f"shape {shape_g} vs {shape_w}"
    if shape_g == "counts":
        bad = sorted(k for k in a.keys() | b.keys() if a.get(k) != b.get(k))
        return None if not bad else "; ".join(f"{k}: {a.get(k)} vs {b.get(k)}" for k in bad)
    if shape_g in ("ints", "labels", "count"):
        if shape_g == "labels":
            a, b = canonical_labels(a), canonical_labels(b)
        if a == b:
            return None
        if shape_g == "count":
            return f"{a} vs {b}"
        bad = [i for i, (x, y) in enumerate(zip(a, b)) if x != y]
        return f"length {len(a)} vs {len(b)}; {len(bad)} differ, first at {bad[:5]}"
    tol = tolerance(kernel)
    if shape_g == "real":
        return None if close(a, b, n, tol) else f"{a!r} vs {b!r}"
    if shape_g == "reals":
        if len(a) != len(b):
            return f"length {len(a)} vs {len(b)}"
        bad = [
            i
            for i, (x, y) in enumerate(zip(a, b))
            if (x is None) != (y is None) or (x is not None and not close(x, y, n, tol))
        ]
        if not bad:
            return None
        i = bad[0]
        return f"{len(bad)} of {len(a)} differ; node {i}: {a[i]!r} vs {b[i]!r}"
    raise ValueError(f"unknown shape {shape_g}")
