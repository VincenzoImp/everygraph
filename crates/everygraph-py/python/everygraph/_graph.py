# SPDX-License-Identifier: Apache-2.0 OR MIT
"""`eg.read`, `eg.open` and `Graph` (`docs/api-sketch.md` § 3.1, § 3.2)."""

from __future__ import annotations

import contextlib
import json
import os
import pathlib
import sys
import time
import warnings
from collections.abc import Callable, Iterable, Iterator
from typing import Any, Literal

from . import _errors, _native, _text
from ._plan import Plan
from ._progress import T_PLAN, Progress
from ._table import Table

Node = str | int | bytes
Size = str | int
ProgressArg = bool | Callable[[dict[str, Any]], None]

_READ_PHASES = ["reading", "identifiers", "building", "checking"]
_MEASURES = ("degree", "in_degree", "out_degree", "pagerank", "core")
_PLANNED = ("pagerank", "components", "kcore", "degrees", "bfs")
_DETAIL: dict[str, Callable[[dict[str, Any]], str]] = {
    "pagerank": lambda m: _text.counted(m["iterations"], "iteration", "iterations"),
    "components": lambda m: _text.counted(m["components"], "component", "components"),
    "kcore": lambda m: f"largest core {m['largest_core']:,}",
    "bfs": lambda m: _text.counted(m["hops"], "hop", "hops"),
}


def _machine() -> tuple[int, int] | None:
    return _native.machine_memory()


def _thread_count(value: int | str) -> int:
    """A number of threads: an `int` of at least 1, or the text of one from
    `EVERYGRAPH_THREADS`."""
    if isinstance(value, str):
        try:
            return _thread_count(int(value))
        except ValueError:
            raise ValueError(f"EVERYGRAPH_THREADS={value!r} is not a number of threads") from None
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("threads must be an int")
    if value < 1:
        raise ValueError(f"threads={value}: at least 1")
    return value


def _threads(value: int | None) -> int:
    """The threads of a call that is not planned: the argument, `EVERYGRAPH_THREADS`, or
    the physical cores the process may use (`docs/api-sketch.md` § 2.4)."""
    if value is None:
        env = os.environ.get("EVERYGRAPH_THREADS")
        return _native.default_threads() if env is None else _thread_count(env)
    return _thread_count(value)


def _memory(value: Size | None, given: str = "memory=") -> tuple[int, str]:
    """The budget in bytes and where it came from: the argument, else
    `EVERYGRAPH_MEMORY`, else half of the memory free now (`docs/api-sketch.md` § 2.4).
    A budget larger than the machine is a `ValueError` (P-16)."""
    machine = _machine()
    origin = given
    if value is None:
        value, origin = os.environ.get("EVERYGRAPH_MEMORY"), "EVERYGRAPH_MEMORY"
    if value is None:
        return (machine[1] // 2 if machine else 8 * 10**9), "default: half of free RAM"
    budget = _text.parse_size(value)
    if machine is not None and budget > machine[0]:
        total, free = machine
        raise ValueError(_errors.text(
            "The memory budget is more than this machine has.",
            f"{given}{value!r} asks for {_text.size(budget)}; this machine has {_text.size(total)} of memory in all.",
            [_errors.Way(f"--memory {_text.typed(free)}", f'memory="{_text.typed(free)}"', "what is free now"),
             _errors.Way("leave out --memory", "leave out memory=",
                         f"the default, half of free RAM: {_text.size(free // 2)} now")],
            "Nothing was changed.",
        ))
    return budget, origin


def _progress_arg(progress: ProgressArg) -> ProgressArg:
    if not (isinstance(progress, bool) or callable(progress)):
        raise TypeError("progress must be True, False or a function of one dict")
    return progress


def _choice(name: str, value: str, options: tuple[str, ...]) -> str:
    if value not in options:
        words = " or ".join(repr(o) for o in options)
        raise ValueError(f"{name} takes {words}, not {value!r}")
    return value


def read(
    data: str | os.PathLike[str],
    *,
    source: str | None = None,
    target: str | None = None,
    weight: str | None = None,
    time: str | None = None,
    directed: bool | None = None,
    self_loops: Literal["drop", "keep"] = "drop",
    duplicates: Literal["events", "collapse", "sum"] = "events",
    ids: Literal["auto", "string"] = "auto",
    out: str | os.PathLike[str] | None = None,
    overwrite: bool = False,
    memory: Size | None = None,
    threads: int | None = None,
    temp_dir: str | os.PathLike[str] | None = None,
    progress: ProgressArg = True,
) -> Graph:
    """Reads a text edge list (CSV, TSV or whitespace; plain, `.gz` or `.zst`) and
    returns its graph. Columns are found by name (`from`/`to`, `source`/`target`) or
    taken in order; the graph is directed when the names say so, unless `directed` is
    given. With `out`, or for an input of 100 MB or more, the graph is written to a
    `.eg` file (beside the input by default) that `eg.open` reopens; a smaller one stays
    in memory. Reading the same input again with the same options opens that file::

        >>> import everygraph as eg
        >>> g = eg.read("friends.csv")
        >>> g.num_nodes, g.num_edges

    `memory`, `threads` and `temp_dir` become the defaults of the graph's calls.
    """
    path = pathlib.Path(data)
    budget, origin = _memory(memory)
    pool = _threads(threads)
    progress = _progress_arg(progress)
    for name, value, options in (
        ("self_loops", self_loops, ("drop", "keep")),
        ("duplicates", duplicates, ("events", "collapse", "sum")),
        ("ids", ids, ("auto", "string")),
    ):
        _choice(name, value, options)
    temp = temp_dir if temp_dir is not None else os.environ.get("EVERYGRAPH_TEMP_DIR")
    asked = {
        "out": None if out is None else str(out),
        "source": source,
        "target": target,
        "weight": weight,
        "time": time,
        "directed": directed,
        "self_loops": self_loops,
        "duplicates": duplicates,
        "ids": ids,
    }
    machine = _machine()
    call = _caller("read", repr(str(data)))
    try:
        seen = _native.inspect(str(path), **asked)
        events = Progress("ingest", _READ_PHASES, seen["job"], progress)
        if progress is True:
            print(_understood(path, seen, asked, budget, origin), file=sys.stderr)
        began = _clock()
        with _stopping(events, "run the same call again", "nothing: no graph was made"):
            native, info = _native.read(
                str(path),
                **asked,
                memory=budget,
                threads=pool,
                temp_dir=None if temp is None else str(temp),
                overwrite=overwrite,
                on_event=events,
            )
    except _native.NativeError as e:
        _errors.raise_native(e, "read", call, written=True, machine_memory=machine[0] if machine else None)
        raise
    graph = Graph(native, memory=memory, threads=threads, temp_dir=temp_dir, info=info)
    if progress is True:
        took = _clock() - began
        if took >= T_PLAN:
            events.flush()
        print(graph._read_summary(path, took), file=sys.stderr)
    return graph


@contextlib.contextmanager
def _stopping(events: Progress, resume: str, written: str) -> Iterator[None]:
    """Sends the stop event, and prints the stop block, before a `KeyboardInterrupt`, or
    an exception of the progress function, leaves the call (G-16, G-18, § 5.4); engine
    errors pass through."""
    try:
        yield
    except _native.NativeError:
        raise
    except KeyboardInterrupt:
        events.stopped("interrupt", resume, written)
        raise
    except Exception:
        events.stopped("exception", resume, written)
        raise


_ID_KINDS = {"integer": "integers", "hex": "hexadecimal integers", "string": "text"}


def _understood(path: pathlib.Path, seen: dict[str, Any], asked: dict[str, Any], budget: int, origin: str) -> str:
    """What `eg.read` understood of its input, before any work (`docs/ux-contract.md`
    § 3.4): the headline, `ids`, `edges`, `columns` and `budget` lines."""
    direction = "directed" if seen["directed"] else "undirected"
    if seen["directed_from"] == "column names":
        direction += " (from the column names)"
    elif seen["directed_from"] == "default":
        direction += " (the column names give no direction)"
    repeated = {"events": "duplicates kept as events", "collapse": "duplicates collapsed",
                "sum": "duplicates summed"}[asked["duplicates"]]
    loops = "self-loops dropped" if asked["self_loops"] == "drop" else "self-loops kept"
    roles = [(seen[role], role) for role in ("source", "target", "weight", "time") if seen[role] is not None]
    used = {name for name, _ in roles}
    ignored = [c for c in seen["header"] or [] if c not in used]
    columns = " · ".join(f"{name} → {role}" for name, role in roles)
    if ignored:
        columns += f" · ignored: {', '.join(ignored)}" if len(ignored) <= 3 else f" · ignored: {len(ignored)} columns"
    compression = "" if seen["compression"] == "plain text" else f" · {seen['compression']}"
    return "\n".join([
        f"{path.name} · {_text.size(seen['bytes'])}{compression}",
        "",
        f"  ids       {_ID_KINDS.get(seen['ids'], seen['ids'])}",
        f"  edges     {direction}",
        f"            {repeated} · {loops}",
        f"  columns   {columns}",
        f"  budget    {_text.size(budget)} memory ({origin})",
        "",
    ])


def _clock() -> float:
    return time.monotonic()


def open(  # noqa: A001 - the approved name (docs/api-sketch.md § 3.1)
    path: str | os.PathLike[str],
    *,
    memory: Size | None = None,
    threads: int | None = None,
    temp_dir: str | os.PathLike[str] | None = None,
) -> Graph:
    """Opens a `.eg` graph file. Reads its header, its table of contents and its index,
    whatever the file's size; raises `CorruptFile` if the quick integrity check fails::

        >>> g = eg.open("friends.eg")
    """
    try:
        _threads(threads)
        native = _native.open(pathlib.Path(path), _memory(memory)[0])
    except _native.NativeError as e:
        _errors.raise_native(e, "open", _caller("open", repr(str(path))))
        raise
    return Graph(native, memory=memory, threads=threads, temp_dir=temp_dir)


def _keys(nodes: Node | Iterable[Node], operation: str) -> list[Any]:
    """The identifiers of a node set (`docs/api-sketch.md` § 2.2) given as identifiers."""
    if isinstance(nodes, (str, int, bytes)):
        return [nodes]
    if hasattr(nodes, "__arrow_c_stream__") or hasattr(nodes, "columns"):
        raise TypeError(f"{operation}: give the nodes as identifiers; tables come with #93")
    return list(nodes)


def _caller(function: str, first: str) -> Callable[[str], str]:
    return lambda kw: f"eg.{function}({first}, {kw})"


class Graph:
    """An immutable graph, in a `.eg` file or in memory. Whole-graph algorithms are
    methods that return a `Table` with one row per node in graph order; each is planned
    before it starts, and takes `memory=`, `threads=`, `dry_run=` and `progress=`
    (`docs/api-sketch.md` § 2.4). On the whole graph of a file the result is kept in the
    file and returned from there the next time (§ 2.8)::

        >>> g = eg.read("friends.csv")
        >>> g.pagerank().top(5)
        >>> g.components().sizes()
        >>> g.explain("kcore", memory="1GB")
    """

    __slots__ = ("_native", "_memory", "_threads", "_temp_dir", "_info")

    def __init__(
        self,
        native: _native.Graph,
        *,
        memory: Size | None,
        threads: int | None,
        temp_dir: str | os.PathLike[str] | None,
        info: dict[str, Any] | None = None,
    ) -> None:
        self._native = native
        self._memory = memory
        self._threads = threads
        self._temp_dir = temp_dir
        self._info = info or {}

    @property
    def num_nodes(self) -> int:
        """The number of nodes."""
        return self._native.num_nodes

    @property
    def num_edges(self) -> int:
        """The number of stored edges: every event counts."""
        return self._native.num_edges

    @property
    def directed(self) -> bool:
        """Whether the graph is directed."""
        return self._native.directed

    @property
    def file(self) -> pathlib.Path | None:
        """The `.eg` file, or `None` for a graph in memory."""
        f = self._native.file
        return None if f is None else pathlib.Path(f)

    def nodes(self) -> Table:
        """One row per node, in graph order: `node`."""
        return Table(self._native.nodes(), self._meta("nodes", {}, 0.0, 0), "in graph order")

    # --- questions about the whole graph ---------------------------------------

    def degrees(
        self,
        nodes: Node | Iterable[Node] | None = None,
        *,
        weight: bool | str = True,
        memory: Size | None = None,
        threads: int | None = None,
        dry_run: bool = False,
        progress: ProgressArg = True,
    ) -> Table | Plan:
        """For every node (or the nodes given, in that order), its number of distinct
        neighbors: `node`, `degree`; on a directed graph `node`, `in_degree`,
        `out_degree`, `degree` (neighbors in either direction). An identifier that is
        not in the graph gives a row of nulls, counted in `meta["unknown"]`
        (`docs/api-sketch.md` § 2.2)::

            >>> g.degrees().top(3, by="degree")
        """
        self._no_weight(weight)
        ids = None if nodes is None else self._native.lookup(_keys(nodes, "degrees"))
        unknown = {} if ids is None else {"unknown": sum(i is None for i in ids)}
        return self._call("degrees", {"weight": weight}, (memory, threads, dry_run), progress,
                          lambda run, e: self._native.degrees(ids, run, e), **unknown)

    def pagerank(
        self,
        *,
        damping: float = 0.85,
        tol: float = 1e-6,
        max_iter: int = 100,
        weight: bool | str = True,
        personalize: Node | Iterable[Node] | None = None,
        precision: Literal["full", "low"] = "full",
        memory: Size | None = None,
        threads: int | None = None,
        dry_run: bool = False,
        progress: ProgressArg = True,
    ) -> Table | Plan:
        """PageRank: `node`, `pagerank`, the scores summing to 1. Stops when the sum of
        absolute changes between two iterations is below `tol` (not scaled by the number
        of nodes, unlike NetworkX), or after `max_iter` iterations; `meta` has
        `iterations` and `converged`. `weight=True` counts the edges between a pair.
        `precision="low"` keeps 32-bit state::

            >>> g.pagerank().top(5)
        """
        self._no_weight(weight)
        _choice("precision", precision, ("full", "low"))
        if not 0.0 <= damping <= 1.0:
            raise ValueError(f"damping={damping}: between 0 and 1")
        if not tol >= 0.0:
            raise ValueError(f"tol={tol}: at least 0")
        if isinstance(max_iter, bool) or not isinstance(max_iter, int) or max_iter < 0:
            raise ValueError(f"max_iter={max_iter!r}: a whole number, at least 0")
        seeds = None if personalize is None else self._ids(personalize, "pagerank")
        if seeds is not None and not seeds:
            raise ValueError("personalize: an empty node set")
        parameters = {"damping": damping, "tol": tol, "max_iter": max_iter, "weight": weight,
                      "personalize": personalize is not None, "precision": precision}
        return self._call("pagerank", parameters, (memory, threads, dry_run), progress,
                          lambda run, e: self._native.pagerank(damping, tol, max_iter, weight is True, seeds,
                                                               precision == "low", run, e))

    def components(
        self,
        *,
        strong: bool = False,
        memory: Size | None = None,
        threads: int | None = None,
        dry_run: bool = False,
        progress: ProgressArg = True,
    ) -> Table | Plan:
        """Weakly connected components, or with `strong=True` the strongly connected ones
        of a directed graph: `node`, `component`, numbered from 0 by decreasing size; of
        two of equal size, the one with the earlier node first::

            >>> g.components().sizes()
        """
        if strong and not self.directed:
            raise _errors.Unsupported(
                "components(strong=True) needs a directed graph.",
                why="An undirected graph has no direction to use: its strong components are its components.",
                ways_forward=[_errors.Way("leave out --strong", "g.components()", "the components")],
                operation="components", reason="there is no direction to use",
            )
        return self._call("components", {"strong": strong}, (memory, threads, dry_run), progress,
                          lambda run, e: self._native.components(strong, run, e))

    def kcore(
        self,
        *,
        memory: Size | None = None,
        threads: int | None = None,
        dry_run: bool = False,
        progress: ProgressArg = True,
    ) -> Table | Plan:
        """The core number of each node, direction ignored: `node`, `core`::

            >>> g.kcore().sizes()
        """
        return self._call("kcore", {}, (memory, threads, dry_run), progress,
                          lambda run, e: self._native.kcore(run, e))

    def bfs(
        self,
        source: Node,
        *,
        direction: Literal["out", "in", "both"] = "out",
        max_hops: int | None = None,
        memory: Size | None = None,
        threads: int | None = None,
        dry_run: bool = False,
        progress: ProgressArg = True,
    ) -> Table | Plan:
        """Hops from `source` to every node: `node`, `distance`, `parent`; null where
        unreachable. `parent` is the earliest node in graph order one hop closer to the
        source::

            >>> g.bfs(0).head()
        """
        _choice("direction", direction, ("out", "in", "both"))
        if max_hops is not None and (isinstance(max_hops, bool) or not isinstance(max_hops, int) or max_hops < 0):
            raise ValueError(f"max_hops={max_hops!r}: a whole number, at least 0")
        found = self._native.lookup([source])[0]
        if found is None:
            raise _errors.UnknownNode(
                f"{source!r} is not a node of this graph.",
                ways_forward=[_errors.Way("check the identifier", "check the identifier", "identifiers are matched exactly; hexadecimal in any case")],
                node=str(source),
            )
        parameters = {"source": source, "direction": direction, "max_hops": max_hops}
        return self._call("bfs", parameters, (memory, threads, dry_run), progress,
                          lambda run, e: self._native.bfs(found, direction, max_hops, run, e))

    def top(self, by: str, n: int = 10, **run: Any) -> Table:
        """The `n` nodes with the highest value of a per-node measure (`"degree"`,
        `"in_degree"`, `"out_degree"`, `"pagerank"`, `"core"`), largest first, ties in
        graph order, with `rank`. The same as `g.pagerank().top(n)` and so on::

            >>> g.top("degree", 100)
        """
        _choice("by", by, _MEASURES)
        if by == "pagerank":
            return self.pagerank(**run).top(n)
        if by == "core":
            return self.kcore(**run).top(n)
        return self.degrees(**run).top(n, by=by)

    def explain(self, operation: str, /, **parameters: Any) -> Plan:
        """How `operation` would run, with these parameters: the same as calling it with
        `dry_run=True`. It never raises a refusal: a plan that does not fit says so::

            >>> g.explain("pagerank", memory="4GB")
        """
        _choice("operation", operation, _PLANNED)
        return getattr(self, operation)(**parameters, dry_run=True)

    # --- plumbing -----------------------------------------------------------

    def _no_weight(self, weight: bool | str) -> None:
        declared = self._native.weight_column
        if isinstance(weight, str) or (weight is True and declared is not None):
            name = weight if isinstance(weight, str) else declared
            raise _errors.Unsupported(
                f"weight={weight!r}: the weight column {name!r} cannot be used yet.",
                ways_forward=[_errors.Way("--no-weight", "weight=False", "every connected pair counts 1")],
                operation="weight", reason="the operations do not read weight columns yet",
            )

    def _ids(self, nodes: Node | Iterable[Node], operation: str) -> list[int]:
        keys = _keys(nodes, operation)
        found = self._native.lookup(keys)
        unknown = [k for k, f in zip(keys, found, strict=True) if f is None]
        if unknown:
            raise _errors.UnknownNode(
                f"{unknown[0]!r} is not a node of this graph"
                + (f" ({len(unknown):,} of the {len(keys):,} given are not)." if len(unknown) > 1 else "."),
                node=str(unknown[0]),
            )
        return [f for f in found if f is not None]

    def _run(self, memory: Size | None, threads: int | None, dry_run: bool) -> tuple[Any, ...]:
        """How a call runs, as the planner takes it: the budget and the most threads,
        each from the argument, the value given when the graph was obtained, the
        environment, or the planner's default, with where it came from (§ 2.4)."""
        if not isinstance(dry_run, bool):
            raise TypeError("dry_run must be True or False")

        def pick(argument: Any, given: Any, variable: str, parse: Callable[[Any], int]) -> tuple[int | None, str]:
            for value, origin in ((argument, "argument"), (given, "graph"), (os.environ.get(variable), "environment")):
                if value is not None:
                    return parse(value), origin
            return None, "default"

        budget = pick(memory, self._memory, "EVERYGRAPH_MEMORY", _text.parse_size)
        pool = pick(threads, self._threads, "EVERYGRAPH_THREADS", _thread_count)
        return (*budget, *pool, dry_run)

    def _meta(self, operation: str, parameters: dict[str, Any], took: float, peak: int, **more: Any) -> dict[str, Any]:
        weight = parameters.get("weight")
        return {
            "operation": operation,
            "parameters": parameters,
            "graph": str(self.file) if self.file else None,
            "view": None,
            "engine": f"everygraph {_native.__version__}",
            "deterministic": True,
            "seed": None,
            "precision": parameters.get("precision", "full") if operation == "pagerank" else None,
            "weight": None if weight in (None, False) else "edges",
            "direction": "ignored" if operation == "kcore" and self.directed else parameters.get("direction"),
            "took_seconds": took,
            "peak_memory_bytes": peak,
            "cached": False,
            "plan": None,
            "truncated": None,
            **more,
        }

    def _call(
        self,
        operation: str,
        parameters: dict[str, Any],
        run: tuple[Size | None, int | None, bool],
        progress: ProgressArg,
        native: Callable[[tuple[Any, ...], Progress], tuple[_native.Table | None, dict[str, Any]]],
        **more: Any,
    ) -> Table | Plan:
        settings = self._run(*run)
        progress = _progress_arg(progress)
        graph = str(self.file) if self.file else f"memory:{self.num_nodes}:{self.num_edges}"
        job = _native.job_id([graph, operation, *(f"{k}={v!r}" for k, v in sorted(parameters.items()))])
        events = Progress(operation, [operation], job, progress, lines=False)
        where = self.file.name if self.file else "this graph"
        unchanged = f"nothing: {where} is unchanged" if self.file else "nothing: the graph is unchanged"
        began = _clock()
        try:
            with _stopping(events, f"g.{operation}(…), the same call again", unchanged):
                table, info = native(settings, events)
        except _native.NativeError as e:
            cls, fields = e.args
            if cls == "Refusal":
                refused = _errors.from_plan(json.loads(fields["document"]), operation, where)
            else:
                machine = _machine()
                try:
                    _errors.raise_native(e, operation, lambda kw: f"g.{operation}({kw})",
                                         machine_memory=machine[0] if machine else None)
                except _errors.EverygraphError as raised:
                    refused = raised
                except ValueError:
                    raise
            if events.plan is not None:
                events.error(refused.to_dict())
            raise refused from None
        took = _clock() - began
        plan = Plan(info.pop("plan"), info.pop("plan_text"))
        if table is None:
            events.result({"path": None, "kind": "none", "bytes": None}, {}, took, 0)
            return plan
        peak = info.pop("peak_memory_bytes", 0)
        unkept = info.pop("unkept", None)
        cached = info.pop("cached", False)
        if unkept is not None:
            message = f"The result could not be kept in {where}: {unkept}."
            continuing = "It is returned, and is computed again next time."
            events.warning("read-only", message, unkept, continuing)
            warnings.warn(f"{message} {continuing}", RuntimeWarning, stacklevel=3)
        meta = self._meta(operation, parameters, took, peak, cached=cached, plan=plan.to_dict(), **info, **more)
        kept = self.file is not None and unkept is None and plan.to_dict()["output"]["kind"] == "result"
        output = {"path": str(self.file), "kind": "result", "bytes": plan.estimate["output_bytes"]["value"]} if kept \
            else {"path": None, "kind": "none", "bytes": None}
        events.result(output, {"nodes": self.num_nodes, "rows": table.num_rows}, took, peak)
        if progress is True and (took >= T_PLAN or cached):
            state = "kept" if cached else _text.duration(took)
            detail = "" if cached else _DETAIL.get(operation, lambda _m: "")(meta)
            print(f"  {operation:<13}{'█' * 16}  {detail:<32}  {state:>11}", file=sys.stderr)
        return Table(table, meta, "in graph order")

    def _read_summary(self, path: pathlib.Path, took: float) -> str:
        where = f"{self.file.name} · {_text.size(self._native.bytes)}" if self.file else f"{path.name} · in memory"
        if self._info.get("cached"):
            return f"{self.file.name} is up to date with {path.name}: opened, not rebuilt.\n  reopen later: eg.open({json.dumps(str(self.file))})"
        lines = [f"✓ {where} · {self.num_nodes:,} nodes · {self.num_edges:,} edges · {_text.duration(took)}"]
        dropped = [
            _text.counted(self._info.get(key, 0), one, many)
            for key, one, many in (("dropped_self_loops", "self-loop", "self-loops"),
                                   ("dropped_duplicates", "duplicate", "duplicates"))
            if self._info.get(key)
        ]
        if dropped:
            lines.append(f"  dropped: {' · '.join(dropped)}")
        if self.file:
            lines.append(f"  reopen later: eg.open({json.dumps(str(self.file))})")
        return "\n".join(lines)

    def __repr__(self) -> str:
        kind = "directed" if self.directed else "undirected"
        if self.file is None:
            return f"Graph in memory · {self.num_nodes:,} nodes · {self.num_edges:,} edges\n  edges     {kind}"
        size = _text.size(self._native.bytes)
        return f"{self.file.name} · {size} · {self.num_nodes:,} nodes · {self.num_edges:,} edges\n  edges     {kind}"
