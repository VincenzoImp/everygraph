# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Progress (`docs/ux-contract.md` § 4): the phase and stop events of § 4.6, handed to a
function given as `progress=`, or printed on standard error as one line per finished
phase."""

from __future__ import annotations

import sys
from collections.abc import Callable
from typing import Any

from . import _text

T_PLAN = 5.0
"""An operation that runs this long prints its phase lines (P-3, a † value)."""

RUNNING_EVERY = 0.25
"""Running events of a phase come at most four times a second (G-17)."""

_UNITS = {"reading": "parts", "identifiers": "identifiers", "building": "parts", "checking": "sections"}
_MADE = {
    "reading": ("row", "rows"),
    "identifiers": ("identifier", "identifiers"),
    "building": ("part", "parts"),
    "checking": ("section verified", "sections verified"),
}


class Progress:
    """The receiver of the engine's events for one operation:
    `(kind, phase, value, t, peak)`, on the calling thread (G-15)."""

    def __init__(
        self,
        operation: str,
        phases: list[str],
        job: str,
        progress: bool | Callable[[dict[str, Any]], None],
        unit: str = "passes",
    ) -> None:
        self.operation = operation
        self.phases = phases
        self.job = job
        self.sink = progress if callable(progress) else None
        self.printing = progress is True
        self.unit = unit
        self.started: dict[str, float] = {}
        self.totals: dict[str, int | None] = {}
        self.done: dict[str, int] = {}
        self.sent: dict[str, float] = {}
        self.current: str | None = None
        self.t = 0.0
        self.peak = 0
        self.lines: list[str] = []
        self.shown = False

    def _event(self, phase: str, state: str, done: int, detail: str | None) -> dict[str, Any]:
        total = self.totals.get(phase)
        fraction = 1.0 if state == "done" else (min(done / total, 1.0) if total else None)
        return {
            "event": "phase",
            "schema": 1,
            "job": self.job,
            "operation": self.operation,
            "t": max(self.t, 0.0),
            "name": phase,
            "index": self.phases.index(phase) if phase in self.phases else 0,
            "count": len(self.phases),
            "state": state,
            "unit": _UNITS.get(phase, self.unit),
            "done": done,
            "total": total,
            "fraction": fraction,
            "pass": None,
            "rate": None,
            "eta_seconds": None,
            "elapsed_seconds": max(self.t - self.started.get(phase, self.t), 0.0),
            "detail": detail,
            "peak_memory_bytes": self.peak,
        }

    def _detail(self, phase: str, made: int) -> str:
        one, many = _MADE.get(phase, ("", ""))
        return _text.counted(made, one, many) if one else ""

    def __call__(self, kind: str, phase: str, value: int | None, t: float, peak: int) -> None:
        self.t, self.peak = t, peak
        if kind == "begin":
            if phase in self.started:
                return
            self.current = phase
            self.started[phase] = self.sent[phase] = t
            self.totals[phase], self.done[phase] = value, 0
            self._send(self._event(phase, "running", 0, None))
        elif kind == "advance":
            self.done[phase] = self.done.get(phase, 0) + (value or 0)
            if t - self.sent.get(phase, 0.0) >= RUNNING_EVERY:
                self.sent[phase] = t
                self._send(self._event(phase, "running", self.done[phase], None))
        else:
            self.current = None
            self._send(self._event(phase, "done", value or 0, self._detail(phase, value or 0)))
            self._line(phase, value or 0)

    def stopped(self, reason: str, call: str) -> None:
        """The stop event (G-16, G-18) before what stopped the call propagates."""
        during = self.current or (self.phases[0] if self.phases else self.operation)
        if self.current is not None:
            self._send(self._event(self.current, "stopped", self.done.get(self.current, 0), None), quiet=True)
        self._send(
            {
                "event": "stopped",
                "schema": 1,
                "job": self.job,
                "operation": self.operation,
                "t": max(self.t, 0.0),
                "stopped": {
                    "reason": reason,
                    "during": during,
                    "fraction": None,
                    "kept": [],
                    "dropped": [p for p in self.phases if p in self.started],
                    "scratch": None,
                    "partial": None,
                    "written": [],
                    "resume": {"python": call, "command": ""},
                    "discard": [],
                },
            },
            quiet=True,
        )

    def _send(self, event: dict[str, Any], quiet: bool = False) -> None:
        if self.sink is None:
            return
        if not quiet:
            self.sink(event)
            return
        try:
            self.sink(event)
        except Exception:  # noqa: BLE001 - the call is already stopping for another reason
            pass

    def _line(self, phase: str, made: int) -> None:
        if not self.printing:
            return
        elapsed = _text.duration(max(self.t - self.started.get(phase, self.t), 0.0))
        detail = self._detail(phase, made)
        self.lines.append(f"  {phase:<13}{'█' * 16}  {detail:<32}  {elapsed:>11}")
        if self.t >= T_PLAN or self.shown:
            self.flush()

    def flush(self) -> None:
        """Prints the lines held back, once the operation has been long enough."""
        if self.lines:
            print("\n".join(self.lines), file=sys.stderr, flush=True)
            self.lines.clear()
        self.shown = True
