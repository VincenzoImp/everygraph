# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Progress (`docs/ux-contract.md` § 4): the phase events of § 4.6, handed to a function
given as `progress=`, or printed on standard error as one line per finished phase."""

from __future__ import annotations

import sys
from collections.abc import Callable
from typing import Any

from . import _text

T_PLAN = 5.0
"""An operation that runs this long prints its phase lines (P-3, a † value)."""

_UNITS = {"reading": "parts", "identifiers": "identifiers", "building": "parts", "checking": "sections"}
_MADE = {
    "reading": ("row", "rows"),
    "identifiers": ("identifier", "identifiers"),
    "building": ("part", "parts"),
    "checking": ("section verified", "sections verified"),
}


class Progress:
    """The receiver of the engine's events for one operation: `(kind, phase, value, t)`,
    on the calling thread (G-15)."""

    def __init__(
        self,
        operation: str,
        phases: list[str],
        job: str,
        progress: bool | Callable[[dict[str, Any]], None],
        unit: str = "passes",
        detail: Callable[[str, int], str] | None = None,
    ) -> None:
        self.operation = operation
        self.phases = phases
        self.job = job
        self.sink = progress if callable(progress) else None
        self.printing = progress is True
        self.unit = unit
        self.detail = detail
        self.started: dict[str, float] = {}
        self.done: dict[str, int] = {}
        self.lines: list[str] = []
        self.shown = False

    def _event(self, phase: str, state: str, t: float, value: int | None, total: int | None) -> dict[str, Any]:
        elapsed = max(t - self.started.get(phase, t), 0.0)
        done = value or 0
        fraction = min(done / total, 1.0) if total else None
        final = state != "running"
        return {
            "event": "phase",
            "schema": 1,
            "job": self.job,
            "operation": self.operation,
            "t": max(t, 0.0),
            "name": phase,
            "index": self.phases.index(phase) if phase in self.phases else 0,
            "count": len(self.phases),
            "state": state,
            "unit": _UNITS.get(phase, self.unit),
            "done": done,
            "total": total,
            "fraction": 1.0 if final else fraction,
            "pass": None,
            "rate": None,
            "eta_seconds": None,
            "elapsed_seconds": elapsed,
            "detail": self._detail(phase, done) if final else None,
            "peak_memory_bytes": 0,
        }

    def _detail(self, phase: str, made: int) -> str:
        if self.detail is not None:
            return self.detail(phase, made)
        one, many = _MADE.get(phase, ("", ""))
        return _text.counted(made, one, many) if one else ""

    def __call__(self, kind: str, phase: str, value: int | None, t: float) -> None:
        if kind == "begin":
            if phase in self.started:
                return
            self.started[phase] = t
            self.done[phase] = 0
            self._send(self._event(phase, "running", t, 0, value))
        elif kind == "advance":
            self.done[phase] = self.done.get(phase, 0) + (value or 0)
        else:
            self._send(self._event(phase, "done", t, value, value))
            self._line(phase, value or 0, t)

    def _send(self, event: dict[str, Any]) -> None:
        if self.sink is not None:
            self.sink(event)

    def _line(self, phase: str, made: int, t: float) -> None:
        if not self.printing:
            return
        elapsed = _text.duration(max(t - self.started.get(phase, t), 0.0))
        detail = self._detail(phase, made)
        self.lines.append(f"  {phase:<13}{'█' * 16}  {detail:<32}  {elapsed:>11}")
        if t >= T_PLAN or self.shown:
            self.flush()

    def flush(self) -> None:
        """Prints the lines held back, once the operation has been long enough."""
        if self.lines:
            print("\n".join(self.lines), file=sys.stderr, flush=True)
            self.lines.clear()
        self.shown = True
