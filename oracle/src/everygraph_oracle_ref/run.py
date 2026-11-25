# SPDX-License-Identifier: Apache-2.0 OR MIT
"""`oracle-ref run CORPUS`: compute every kernel of every graph with each library and
write reference.json next to the graph, recording where the libraries disagree.

Memory is bounded twice. A graph with more rows than `max_rows` is not loaded at all:
reference implementations are for small and medium graphs, and large graphs are checked
by invariants. Every worker process also runs under an address-space limit
(RLIMIT_AS, `--max-memory-gib` per job), so a kernel that would grow beyond it fails
with MemoryError, which is recorded as an error of that graph and kernel, instead of
taking the machine's memory. The summary reports the largest resident set of any worker.
"""

from __future__ import annotations

import json
import os
import resource
import time
import traceback
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

from .compare import compare
from .graph import project, read_entry
from . import refs
from .refs import ADJUSTMENTS, compute

REFERENCE_FORMAT = "everygraph-oracle-reference"
# Must equal PYTHON_MAX_NODES in crates/everygraph-oracle/src/corpus.rs.
PYTHON_MAX_NODES = 1 << 20


def versions() -> dict[str, str]:
    import igraph
    import networkx
    import numpy
    import scipy

    return {
        "networkx": networkx.__version__,
        "igraph": igraph.__version__,
        "scipy": scipy.__version__,
        "numpy": numpy.__version__,
    }


_LIMITS = {"max_rows": 2_000_000, "memory_gib": 3.0}


def _init_worker(memory_gib: float, max_rows: int) -> None:
    _LIMITS["max_rows"] = max_rows
    _LIMITS["memory_gib"] = memory_gib
    limit = int(memory_gib * (1 << 30))
    resource.setrlimit(resource.RLIMIT_AS, (limit, limit))
    os.nice(10)


def _peak_rss_mib() -> float:
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024


def one(d: str) -> dict:
    d = Path(d)
    rows = json.loads((d / "graph.json").read_text())["rows"]
    if rows > _LIMITS["max_rows"]:
        reason = f"skipped: {rows} rows > max_rows {_LIMITS['max_rows']} (large graphs are checked by invariants only)"
        return {"name": d.name, "kernels": 0, "disagreements": [], "errors": [{"kernel": None, "error": reason}], "peak_mib": _peak_rss_mib()}
    n = json.loads((d / "graph.json").read_text())["n"]
    if n > PYTHON_MAX_NODES:
        # Sparse-id graphs: per-node answers would make reference.json hundreds of MB;
        # they are checked by the in-process references and the invariants only.
        doc = {"format": REFERENCE_FORMAT, "version": 1, "libraries": versions(), "results": [],
               "skipped": f"{n} nodes > {PYTHON_MAX_NODES}: checked in Rust only"}
        (d / "reference.json").write_text(json.dumps(doc) + "\n")
        return {"name": d.name, "kernels": 0, "disagreements": [], "errors": [], "peak_mib": _peak_rss_mib(), "skipped": True}
    g = read_entry(d)
    p = project(g)
    results, disagreements, errors = [], [], []
    for kernel in g.kernels:
        try:
            values = compute(p, g.partition, kernel)
        except MemoryError:
            errors.append({"kernel": kernel, "error": f"skipped: exceeded the per-job memory limit of {_LIMITS['memory_gib']} GiB"})
            continue
        except Exception:  # a library failing is a finding, not a crash of the runner
            errors.append({"kernel": kernel, "error": traceback.format_exc(limit=3)})
            continue
        agree = True
        libs = list(values)
        for lib in libs[1:]:
            diff = compare(kernel["kernel"], g.n, values[lib], values[libs[0]])
            if diff is not None:
                agree = False
                disagreements.append({"kernel": kernel, "libraries": [lib, libs[0]], "detail": diff})
        entry = {"kernel": kernel, "values": values, "agree": agree}
        if kernel["kernel"] == "pagerank":
            entry["networkx_tol"] = refs.LAST_NX_TOL.get("tol")
        results.append(entry)
    pairs = len(p.arcs)
    touched = {v for pair in p.undirected for v in pair}
    if p.keep_loops:
        touched.update(p.loop_nodes)
    isolates = g.n - len(touched)
    doc = {
        "format": REFERENCE_FORMAT,
        "version": 1,
        "libraries": versions(),
        "adjustments": ADJUSTMENTS,
        "ingest": {
            "rows": len(g.src),
            "self_loop_rows": p.self_loop_rows,
            "duplicate_rows": p.duplicate_rows,
            "pairs": pairs,
            "isolates": isolates,
        },
        "results": results,
        "disagreements": disagreements,
        "errors": errors,
    }
    (d / "reference.json").write_text(json.dumps(doc, separators=(",", ":"), allow_nan=False) + "\n")
    return {"name": g.name, "kernels": len(results), "disagreements": disagreements, "errors": errors, "peak_mib": _peak_rss_mib()}


def run(corpus: Path, jobs: int, memory_gib: float = 3.0, max_rows: int = 2_000_000) -> int:
    dirs = sorted(str(d) for d in corpus.iterdir() if (d / "graph.json").is_file())
    if not dirs:
        print(f"no graphs under {corpus}")
        return 2
    t0 = time.monotonic()
    with ProcessPoolExecutor(max_workers=jobs, initializer=_init_worker, initargs=(memory_gib, max_rows)) as ex:
        summaries = list(ex.map(one, dirs, chunksize=4))
    peak = max(summaries, key=lambda s: s["peak_mib"])
    kernels = sum(s["kernels"] for s in summaries)
    rust_only = sum(1 for s in summaries if s.get("skipped"))
    dis = [(s["name"], x) for s in summaries for x in s["disagreements"]]
    errs = [(s["name"], x) for s in summaries for x in s["errors"]]
    print(
        f"oracle-ref: {len(dirs)} graphs ({rust_only} above {PYTHON_MAX_NODES} nodes left to Rust), {kernels} kernel results, {len(dis)} disagreements between libraries, "
        f"{len(errs)} errors (skipped cases included), {time.monotonic() - t0:.1f} s, {jobs} processes; "
        f"largest worker resident set {peak['peak_mib']:.0f} MiB ({peak['name']}; limit {memory_gib} GiB of address space per job)"
    )
    for name, x in dis[:40]:
        print(f"  disagree {name} {x['kernel']} {x['libraries']}: {x['detail']}")
    for name, x in errs[:20]:
        print(f"  error {name} {x['kernel']}: {x['error'].strip().splitlines()[-1]}")
    return 0 if not errs else 1
