# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The exceptions and the warning of `docs/api-sketch.md` § 3.10, whose text follows the
pattern of `docs/ux-contract.md` § 6.1: what; why, with the arithmetic; the ways
forward as code that can be copied; and what state things were left in."""

from __future__ import annotations

import dataclasses
from collections.abc import Callable, Mapping
from typing import Any

from . import _text


@dataclasses.dataclass(frozen=True)
class Way:
    """One way forward: what the command line shows, the same as a Python call, its
    consequence (possibly empty), and the bytes it needs when that is known."""

    command: str
    python: str
    note: str = ""
    needs_bytes: int | None = None

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {"command": self.command, "python": self.python, "note": self.note}
        if self.needs_bytes is not None:
            out["needs_bytes"] = self.needs_bytes
        return out


class EverygraphError(Exception):
    """Every error raised by the engine. `str()` is the whole message; the same content
    is in the attributes, and `to_dict()` is the error document of
    `docs/ux-contract.md` § 6.1."""

    def __init__(
        self,
        message: str,
        *,
        why: str | None = None,
        arithmetic: Mapping[str, object] | None = None,
        ways_forward: list[Way] | None = None,
        changed: bool = False,
        written: bool = False,
        kept: Mapping[str, object] | None = None,
        machine_memory_bytes: int | None = None,
        **particular: Any,
    ) -> None:
        self.message = message
        self.why = why
        self.arithmetic = arithmetic
        self.ways_forward = list(ways_forward or [])
        self.changed = changed
        self.kept = kept
        self.machine_memory_bytes = machine_memory_bytes
        self._particular = particular
        self._written = written
        for key, value in particular.items():
            setattr(self, key, value)
        super().__init__(self._text())

    def _text(self) -> str:
        lines = [self.message]
        if self.why:
            lines += ["", *self.why.splitlines()]
        if self.ways_forward:
            lines += ["", "  Ways forward:"]
            for way in self.ways_forward:
                if way.note:
                    lines.append(f"    {way.python:<32}  {way.note}")
                else:
                    lines.append(f"    {way.python}")
        lines += ["", "Nothing was written." if self._written else "Nothing was changed."]
        return "\n".join(lines)

    def __str__(self) -> str:
        return self._text()

    def to_dict(self) -> dict[str, Any]:
        doc: dict[str, Any] = {
            "schema": 1,
            "error": type(self).__name__,
            "message": self.message,
            "ways_forward": [w.to_dict() for w in self.ways_forward],
            "changed": self.changed,
        }
        if self.why:
            doc["why"] = self.why
        if self.arithmetic is not None:
            doc["arithmetic"] = dict(self.arithmetic)
        if self.machine_memory_bytes is not None:
            doc["machine_memory_bytes"] = self.machine_memory_bytes
        if self.kept is not None:
            doc["kept"] = dict(self.kept)
        doc.update(self._particular)
        return doc


class InsufficientBudget(EverygraphError):
    """The state of an operation does not fit the memory budget. Raised before any work."""


class InsufficientDisk(EverygraphError):
    """The scratch or output directory lacks room for the plan."""


class InvalidInput(EverygraphError):
    """The input cannot be read as described, or its data cannot be used as asked."""


class InvalidExpression(EverygraphError, ValueError):
    """The text of a filter does not parse, or names a column the graph lacks."""


class UnknownNode(EverygraphError, LookupError):
    """A call about one node names an identifier that is not in the graph."""


class Unsupported(EverygraphError):
    """The operation is not defined for this graph, or is above a size guard."""


class OutputExists(EverygraphError, FileExistsError):
    """The output path holds a file that Everygraph may not replace."""


class CorruptFile(EverygraphError):
    """A graph file fails its integrity checks, or was written by a format this version
    cannot read."""


class TruncatedWarning(UserWarning):
    """A result was cut short by its `limit` or its `timeout` (`docs/api-sketch.md`
    § 2.5)."""


def raise_native(
    error: BaseException,
    operation: str,
    call: Callable[[str], str],
    *,
    written: bool = False,
    machine_memory: int | None = None,
) -> None:
    """Raises the exception of a `NativeError(class, fields)` from `operation`; `call`
    gives the Python call of a way forward from its arguments (`memory="6GB"`)."""
    cls, fields = error.args
    f = dict(fields)
    common = {"written": written, "machine_memory_bytes": machine_memory}
    if cls == "ValueError":
        raise ValueError(f["message"]) from None
    if cls == "InsufficientBudget":
        budget = f["budget_bytes"]
        smallest = budget - f["available_bytes"] + f["needs_bytes"]
        fits = machine_memory is None or smallest <= machine_memory
        ways = []
        if fits:
            kw = f'memory="{_text.typed(smallest)}"'
            ways.append(Way(f"--memory {_text.typed(smallest)}", call(kw), "a budget that holds this step", smallest))
        else:
            phrase = f"a machine with at least {_text.size(smallest)} of memory"
            ways.append(Way(phrase, phrase))
        if f.get("threads", 1) > 1:
            ways.append(Way("--threads 1", call("threads=1"), "fewer blocks in flight need less memory"))
        raise InsufficientBudget(
            f"{operation}() needs {_text.size(f['needs_bytes'])} for {f['step']}; "
            f"the budget is {_text.size(budget)}.",
            why=f"{_text.size(f['available_bytes'])} of the budget was left when it was asked for.",
            ways_forward=ways,
            step=f["step"],
            needs_bytes=f["needs_bytes"],
            budget_bytes=budget,
            smallest_budget_bytes=smallest if fits else None,
            **common,
        ) from None
    if cls == "InvalidInput":
        problems = f.get("problems", [])
        if problems:
            n = f["invalid_rows"]
            what = f"{_text.counted(n, 'line is', 'lines are')} not valid rows"
            why = "\n".join(f"{p['file']}:{p['line']}: {p['problem']}\n  {p['excerpt']}" for p in problems)
            fix = "correct that line" if n == 1 else f"correct those {_text.count(n)} lines"
            ways = [Way(f"{fix}, then run the same command again", f"{fix}, then run the same call again")]
            extra = {"problems": problems, "invalid_rows": n, "rows_checked": f["rows_checked"]}
        else:
            what = f["reason"]
            why = f.get("examples") and f"For example: {f['examples']}."
            ways = []
            if "examples" in f:
                ways = [Way("--ids string", call('ids="string"'), "read every identifier as text")]
            extra = {"column": "source and target"}
        raise InvalidInput(f"Cannot read {f['file']}: {what}.", why=why, ways_forward=ways, **extra, **common) from None
    if cls == "CorruptFile":
        path = f.get("path", "the graph")
        raise CorruptFile(
            f"{path} is damaged: {f['why']}.",
            ways_forward=[Way(f"everygraph verify {path}", f"everygraph verify {path}", "list every problem")],
            path=path,
            section=f.get("section"),
            **common,
        ) from None
    if cls == "OutputExists":
        holds = "another graph" if f["holds"] == "other graph" else "a file that Everygraph did not write"
        raise OutputExists(
            f"{f['path']} exists and holds {holds}.",
            ways_forward=[
                Way("--out NEW.eg", call('out="NEW.eg"'), "write the graph under another name"),
                Way("--overwrite", call("overwrite=True"), "replace it"),
            ],
            path=f["path"],
            holds=f["holds"],
            **common,
        ) from None
    raise EverygraphError(f.get("message", str(fields)), **common) from None
