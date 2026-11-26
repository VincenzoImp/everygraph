# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Everygraph: graphs larger than memory, on one machine.

    >>> import everygraph as eg
    >>> g = eg.read("friends.csv")       # a text edge list; .gz and .zst too
    >>> g.pagerank().top(5)              # an Arrow table: Polars, pandas, DuckDB read it
    >>> g.components().sizes()

The names, parameters and results are those of `docs/api-sketch.md` § 3.
"""

from ._errors import (
    CorruptFile,
    EverygraphError,
    InsufficientBudget,
    InsufficientDisk,
    InvalidExpression,
    InvalidInput,
    OutputExists,
    TruncatedWarning,
    UnknownNode,
    Unsupported,
    Way,
)
from ._graph import Graph, open, read
from ._native import __version__
from ._table import Table

__all__ = [
    "CorruptFile",
    "EverygraphError",
    "Graph",
    "InsufficientBudget",
    "InsufficientDisk",
    "InvalidExpression",
    "InvalidInput",
    "OutputExists",
    "Table",
    "TruncatedWarning",
    "UnknownNode",
    "Unsupported",
    "Way",
    "__version__",
    "open",
    "read",
]
