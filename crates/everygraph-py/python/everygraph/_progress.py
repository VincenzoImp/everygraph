# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Progress (`docs/ux-contract.md` § 4): the events of § 4.6, handed to a function given
as `progress=`, or printed on standard error: the plan block of a long operation, one
line per finished phase, and the stop block (§ 5.4). An operation sends one `plan` event
first and one `result`, `stopped` or `error` event last (G-16)."""

from __future__ import annotations

import json
import sys
from collections.abc import Callable
from typing import Any

from . import _plan, _text

T_PLAN = 5.0
"""An operation that runs this long prints its plan block and phase lines (P-3, a †
value)."""

RUNNING_EVERY = 0.25
"""Running events of a phase come at most four times a second (G-17)."""

_UNITS = {"reading": "parts", "identifiers": "identifiers", "building": "parts", "checking": "sections"}
_MADE = {
    "reading": ("row", "rows"),
    "identifiers": ("identifier", "identifiers"),
    "building": ("part", "parts"),
    "checking": ("section verified", "sections verified"),
}
_ESTIMATES = ("seconds", "peak_memory_bytes", "scratch_bytes", "output_bytes")


class Progress:
    """The receiver of the engine's events for one operation:
    `(kind, phase, value, t, peak)`, on the calling thread (G-15). A `plan` event
    carries the plan document in place of the phase. Without `lines`, the caller prints
    the phase lines itself."""

    def __init__(
        self,
        operation: str,
        phases: list[str],
        job: str,
        progress: bool | Callable[[dict[str, Any]], None],
        unit: str = "passes",
        lines: bool = True,
    ) -> None:
        self.operation = operation
        self.phase_lines = lines
        self.phases = phases
        self.job = job
        self.sink = progress if callable(progress) else None
        self.printing = progress is True
        self.unit = unit
        self.started: dict[str, float] = {}
        self.totals: dict[str, int | None] = {}
        self.done: dict[str, int] = {}
        self.sent: dict[str, float] = {}
        self.finished: list[str] = []
        self.current: str | None = None
        self.plan: dict[str, Any] | None = None
        self.t = 0.0
        self.peak = 0
        self.lines: list[str] = []
        self.shown = False

    def _head(self, kind: str) -> dict[str, Any]:
        return {"event": kind, "schema": 1, "job": self.job, "operation": self.operation, "t": max(self.t, 0.0)}

    def _event(self, phase: str, state: str, done: int, detail: str | None) -> dict[str, Any]:
        total = self.totals.get(phase)
        fraction = 1.0 if state in ("done", "kept") else (min(done / total, 1.0) if total else None)
        return {
            **self._head("phase"),
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
        if kind == "plan":
            self.plan = json.loads(phase)
            self.phases = [p["name"] for p in self.plan["phases"]] or self.phases
            self._send({**self._head("plan"), "plan": self.plan})
            if self.printing and self.plan["estimate"]["seconds"]["value"] >= T_PLAN:
                print(_plan.block(self.plan), file=sys.stderr, flush=True)
                self.shown = True
        elif kind == "begin":
            if phase in self.started:
                return
            if self.current is not None:
                self._finish(self.current, self.done.get(self.current, 0))
            self.current = phase
            self.started[phase] = self.sent[phase] = t
            self.totals[phase], self.done[phase] = value, 0
            self._send(self._event(phase, "running", 0, None))
        elif kind == "advance":
            self.done[phase] = self.done.get(phase, 0) + (value or 0)
            if t - self.sent.get(phase, 0.0) >= RUNNING_EVERY:
                self.sent[phase] = t
                self._send(self._event(phase, "running", self.done[phase], None))
        elif kind == "kept":
            self.finished.append(phase)
            self._send(self._event(phase, "kept", 0, None))
            self._line(phase, "kept")
        else:
            self._finish(phase, value or 0)

    def _finish(self, phase: str, made: int) -> None:
        self.current = None
        self.finished.append(phase)
        self._send(self._event(phase, "done", made, self._detail(phase, made)))
        elapsed = _text.duration(max(self.t - self.started.get(phase, self.t), 0.0))
        self._line(phase, elapsed, self._detail(phase, made))

    def result(self, output: dict[str, Any], counts: dict[str, int], took: float, peak: int,
               dropped: dict[str, int] | None = None) -> None:
        """The `result` event (G-16): what was made, and the actual figures next to the
        plan's estimates (P-9)."""
        actual = {"seconds": took, "peak_memory_bytes": peak, "scratch_bytes": 0,
                  "output_bytes": output.get("bytes") or 0}
        estimate = (self.plan or {}).get("estimate", {})
        errors = {}
        for key in _ESTIMATES:
            value = estimate.get(key, {}).get("value", 0)
            got = actual[key]
            relative = (got - value) / got if got else (0.0 if value == 0 else None)
            errors[key] = {"relative": relative, "absolute": value - got}
        self._send({
            **self._head("result"),
            "output": output,
            "counts": counts,
            "dropped": dropped or {},
            "elapsed_seconds": took,
            "peak_memory_bytes": peak,
            "scratch_bytes": 0,
            "estimate_error": errors,
            "next": [],
        })

    def error(self, document: dict[str, Any]) -> None:
        """The `error` event (G-16), with the error document of § 6.1, after the final
        state of a phase that was running (G-17)."""
        if self.current is not None:
            self._send(self._event(self.current, "stopped", self.done.get(self.current, 0), None), quiet=True)
            self.current = None
        self._send({**self._head("error"), "error": document}, quiet=True)

    def warning(self, code: str, message: str, why: str | None, continuing: str | None) -> None:
        """A `warning` event of a stable `code` (§ 6.2)."""
        self._send({**self._head("warning"), "code": code, "message": message, "why": why,
                    "ways_forward": [], "continuing": continuing})

    def stopped(self, reason: str, call: str, written: str) -> None:
        """The stop event (G-16, G-18) before what stopped the call propagates; with
        `progress=True`, the stop block of § 5.4, whose `written` line is `written`."""
        during = self.current or (self.phases[0] if self.phases else self.operation)
        if self.current is not None:
            self._send(self._event(self.current, "stopped", self.done.get(self.current, 0), None), quiet=True)
        dropped = [p for p in self.phases if p in self.started and p not in self.finished]
        self._send(
            {
                **self._head("stopped"),
                "stopped": {
                    "reason": reason,
                    "during": during,
                    "fraction": None,
                    "kept": [],
                    "dropped": dropped,
                    "scratch": None,
                    "partial": None,
                    "written": [],
                    "resume": {"python": call, "command": ""},
                    "discard": [],
                },
            },
            quiet=True,
        )
        if self.printing:
            first = ("Stopped at your request" if reason == "interrupt"
                     else "Stopped by an exception") + f", during {during}."
            finished = ", ".join(self.finished) or "nothing"
            print("\n".join([
                first, "",
                f"  kept      {finished}",
                f"  dropped   {during} — that phase starts again from its beginning",
                f"  written   {written}", "",
                "  continue:  run the same call again",
            ]), file=sys.stderr, flush=True)

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

    def _line(self, phase: str, right: str, detail: str = "") -> None:
        if not (self.printing and self.phase_lines):
            return
        self.lines.append(f"  {phase:<13}{'█' * 16}  {detail:<32}  {right:>11}")
        if self.t >= T_PLAN or self.shown:
            self.flush()

    def flush(self) -> None:
        """Prints the lines held back, once the operation has been long enough."""
        if self.lines:
            print("\n".join(self.lines), file=sys.stderr, flush=True)
            self.lines.clear()
        self.shown = True
