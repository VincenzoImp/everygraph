# SPDX-License-Identifier: Apache-2.0 OR MIT
"""`Table`: a result that is an Arrow table (`docs/api-sketch.md` § 3.4)."""

from __future__ import annotations

import json
import types
from collections.abc import Mapping
from typing import Any

from . import _native

_SHOWN = 10
"""A table of more rows shows its first five, a row of `…`, and its last."""
_WIDTH = 80


class Table:
    """A result: one row per node (or per value, or per neighbor), in a defined order
    (`docs/api-sketch.md` § 2.12). It implements the Arrow PyCapsule interface, so
    Polars, pandas, pyarrow and DuckDB read it directly, the numeric columns without a
    copy::

        >>> pr = g.pagerank()
        >>> import polars as pl
        >>> pl.from_arrow(pr).sort("pagerank", descending=True).head(3)
    """

    __slots__ = ("_native", "_meta", "_order", "_of")

    def __init__(
        self, native: _native.Table, meta: Mapping[str, Any], order: str, of: int | None = None
    ) -> None:
        self._native = native
        self._meta = types.MappingProxyType(dict(meta))
        self._order = order
        self._of = of

    @property
    def columns(self) -> list[str]:
        """The column names, in order."""
        return self._native.columns

    @property
    def num_rows(self) -> int:
        """The number of rows."""
        return self._native.num_rows

    def __len__(self) -> int:
        return self._native.num_rows

    @property
    def truncated(self) -> bool:
        """Whether the result was cut short by a `limit` or a `timeout` (§ 2.5)."""
        return self._meta.get("truncated") is not None

    @property
    def meta(self) -> Mapping[str, object]:
        """What made the table: the operation, its parameters, the graph, how long it
        took, the peak memory, and what the operation adds (`iterations`, …)."""
        return self._meta

    def _derived(self, native: _native.Table, order: str, of: int | None = None) -> Table:
        return Table(native, self._meta, order, of)

    def head(self, n: int = 5) -> Table:
        """The first `n` rows."""
        of = self._of if self._of is not None else self.num_rows
        return self._derived(self._native.head(_count(n, "n")), self._order, of)

    def top(self, n: int = 10, *, by: str | None = None) -> Table:
        """The `n` rows with the largest value of `by` (default: the last numeric
        column), largest first, with a `rank` column from 1. Ties keep the table's
        order; rows whose value is null are left out::

            >>> g.pagerank().top(5)
        """
        column = by if by is not None else self.columns[-1]
        native = self._native.top(_count(n, "n"), column)
        return self._derived(native, f"top {n:,} of {self.num_rows:,} rows by {column}")

    def sizes(self) -> Table:
        """For a table that assigns each node to a component or a core: one row per
        value with its number of nodes, largest first."""
        column = self.columns[-1]
        return self._derived(self._native.sizes(column), "sorted by nodes, largest first")

    def to_arrow(self) -> Any:
        """The table as a `pyarrow.Table` (needs pyarrow)."""
        import pyarrow as pa

        return pa.table(self)

    def to_pandas(self) -> Any:
        """The table as a `pandas.DataFrame` (needs pyarrow and pandas)."""
        return self.to_arrow().to_pandas()

    def __arrow_c_schema__(self) -> object:
        return self._native.schema_capsule(self._meta_json())

    def __arrow_c_stream__(self, requested_schema: object | None = None) -> object:
        return self._native.stream_capsule(self._meta_json())

    def _meta_json(self) -> str:
        return json.dumps(dict(self._meta), default=str)

    def __repr__(self) -> str:
        return _grid(self)


def _count(n: int, name: str) -> int:
    if isinstance(n, bool) or not isinstance(n, int):
        raise TypeError(f"{name} must be an int")
    if n < 0:
        raise ValueError(f"{name}={n}: a number of rows is at least 0")
    return n


def _cell(value: object) -> tuple[str, bool]:
    """The text of a value, and whether it is a number (right-aligned)."""
    if value is None:
        return "", False
    if isinstance(value, bool):
        return str(value), False
    if isinstance(value, int):
        return f"{value:,}", True
    if isinstance(value, float):
        return f"{value:.8g}" if abs(value) >= 1e-4 or value == 0 else f"{value:.6e}", True
    if isinstance(value, bytes):
        return "0x" + value.hex(), False
    return str(value), False


def _shorten(text: str, width: int) -> str:
    if len(text) <= width:
        return text
    keep = max(width - 1, 2)
    return text[: (keep + 1) // 2] + "…" + text[len(text) - keep // 2 :]


def _grid(t: Table) -> str:
    """The grid of `docs/api-sketch.md` § 2.6: a header, the rows (first five, `…`,
    last, for more than ten), and one footer line."""
    n = t.num_rows
    if n > _SHOWN:
        rows = [*t._native.rows(0, 5), None, *t._native.rows(n - 1, n)]
    else:
        rows = list(t._native.rows(0, n))
    cells = [[_cell(v) for v in row] if row is not None else None for row in rows]
    names = t.columns
    widths = [len(name) for name in names]
    numeric = [False] * len(names)
    for row in cells:
        for i, (text, number) in enumerate(row or []):
            widths[i] = max(widths[i], len(text))
            numeric[i] = numeric[i] or number
    over = sum(widths) + 3 * len(widths) + 1 - _WIDTH
    for i in range(len(names)):
        if over > 0 and not numeric[i] and widths[i] > 12:
            cut = min(over, widths[i] - 12)
            widths[i] -= cut
            over -= cut

    def line(left: str, mid: str, right: str) -> str:
        return left + mid.join("─" * (w + 2) for w in widths) + right

    def text_row(values: list[tuple[str, bool]]) -> str:
        parts = []
        for (text, number), w in zip(values, widths, strict=True):
            text = _shorten(text, w)
            parts.append(text.rjust(w) if number else text.ljust(w))
        return "│ " + " │ ".join(parts) + " │"

    out = [line("┌", "┬", "┐"), text_row([(name, False) for name in names]), line("├", "┼", "┤")]
    for row in cells:
        if row is None:
            out.append(text_row([("…", numeric[i]) for i in range(len(names))]))
        else:
            out.append(text_row(row))
    out.append(line("└", "┴", "┘"))
    shown = len([r for r in cells if r is not None])
    total = t._of if t._of is not None else n
    if t._order.startswith("top ") and n <= _SHOWN:
        out.append(t._order)
    elif shown < total:
        out.append(f"{shown:,} of {total:,} rows shown · {t._order}")
    else:
        out.append(f"{n:,} {'row' if n == 1 else 'rows'} · {t._order}")
    return "\n".join(out)
