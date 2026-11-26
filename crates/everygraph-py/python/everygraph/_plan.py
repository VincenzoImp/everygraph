# SPDX-License-Identifier: Apache-2.0 OR MIT
"""`Plan` (`docs/api-sketch.md` § 3.9): what an operation would do and cost, or why it
cannot, made by the engine's planner before any work. Its document is the plan document
of `docs/ux-contract.md` § 3.6; its text is the step table of § 3.4."""

from __future__ import annotations

import json
import types
from collections.abc import Mapping
from typing import Any

from . import _text

_REGIMES = {"in_memory": "in memory", "mapped": "edges on disk, read as needed",
            "streamed": "edges on disk, read in passes"}
_ORIGINS = {"argument": "memory=", "graph": "set when the graph was opened",
            "environment": "EVERYGRAPH_MEMORY", "default": "default: half of free RAM"}


class Plan:
    """What `g.explain(...)` and `dry_run=True` return. A value: making it ran nothing
    and wrote nothing. A plan that does not fit is still a plan: `fits` is `False` and
    `refusal` holds what the exception would have said::

        >>> g.explain("pagerank", memory="4GB")
        >>> g.pagerank(memory="4GB", dry_run=True).fits
    """

    __slots__ = ("_doc", "_text")

    def __init__(self, document: str, text: str) -> None:
        self._doc = json.loads(document)
        self._text = text

    @property
    def operation(self) -> str:
        """The operation planned."""
        return self._doc["operation"]

    @property
    def fits(self) -> bool:
        """Whether it can run within its budget."""
        return self._doc["fits"]

    @property
    def estimate(self) -> Mapping[str, object]:
        """`seconds`, `peak_memory_bytes`, `scratch_bytes` and `output_bytes`, each a value
        with its range and basis (`docs/ux-contract.md` P-7)."""
        return types.MappingProxyType(self._doc["estimate"])

    @property
    def refusal(self) -> Mapping[str, object] | None:
        """The error document of a plan that does not fit, else `None`."""
        refusal = self._doc["refusal"]
        return None if refusal is None else types.MappingProxyType(refusal)

    def to_dict(self) -> dict[str, Any]:
        """The plan document (`docs/ux-contract.md` § 3.6)."""
        return json.loads(json.dumps(self._doc))

    def __repr__(self) -> str:
        return self._text

    def __str__(self) -> str:
        return self._text


def block(doc: Mapping[str, Any]) -> str:
    """The headline and the plan block printed before a long operation on a graph
    (`docs/ux-contract.md` § 3.4): `machine`, `budget`, `plan`, `estimate`."""
    g, m, b, r, e = doc["graph"], doc["machine"], doc["budget"], doc["regime"], doc["estimate"]
    name = g["path"].replace("\\", "/").rsplit("/", 1)[-1] if g["path"] else "the graph"
    head = f"{name} · {_text.size(g['bytes'])} · {g['nodes']:,} nodes · {g['edges']:,} edges"
    machine = f"{m['cores']} cores"
    if m["memory_bytes"]:
        free = m["memory_free_bytes"]
        limit = m["memory_limit_bytes"]
        room = (f"{_text.size(limit)} allowed by the container" if limit is not None and limit < m["memory_bytes"]
                else f"{_text.size(free)} free")
        machine += f" · {_text.size(m['memory_bytes'])} RAM ({room})"
    storage = m["storage"]
    if storage["kind"] != "unknown":
        machine += f" · {storage['kind']}, {_text.size(storage['free_bytes'])} free"
    lines = [head, "", f"  machine   {machine}",
             f"  budget    {_text.size(b['memory_bytes'])} memory ({_ORIGINS[b['memory_from']]})"]
    if b["above_free"]:
        lines.append(f"            more than the {_text.size(m['memory_free_bytes'])} free now: the system may swap")
    lines.append(f"  plan      {_REGIMES[r['name']]} — {r['reason']}")
    peak = e["peak_memory_bytes"]["value"]
    output = e["output_bytes"]["value"]
    estimate = f"{about(e['seconds']['value'])} · peak {_text.size(peak)} · no scratch"
    if output:
        estimate += f" · result {_text.size(output)} in {name}"
    lines.append(f"  estimate  {estimate}")
    return "\n".join(lines) + "\n"


def about(seconds: float) -> str:
    """A time whose interval is wide: "about" and two significant digits (P-8), as the
    step table writes it."""
    def rounded(x: float) -> str:
        return f"{x:.0f}" if x >= 10 else f"{x:.1f}"
    if seconds < 1:
        return f"about {rounded(seconds * 1000)} ms"
    if seconds < 120:
        return f"about {rounded(seconds)} s"
    if seconds < 7200:
        return f"about {rounded(seconds / 60)} min"
    return f"about {rounded(seconds / 3600)} h"
