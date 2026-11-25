# SPDX-License-Identifier: Apache-2.0 OR MIT
"""`oracle-ref run CORPUS`: compute every kernel of every graph with each library and
write reference.json next to the graph, recording where the libraries disagree."""

from __future__ import annotations

import json
import os
import time
import traceback
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

from .compare import compare
from .graph import project, read_entry
from .refs import ADJUSTMENTS, compute

REFERENCE_FORMAT = "everygraph-oracle-reference"


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


def one(d: str) -> dict:
    d = Path(d)
    g = read_entry(d)
    p = project(g)
    results, disagreements, errors = [], [], []
    for kernel in g.kernels:
        try:
            values = compute(p, g.partition, kernel)
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
        results.append({"kernel": kernel, "values": values, "agree": agree})
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
    return {"name": g.name, "kernels": len(results), "disagreements": disagreements, "errors": errors}


def run(corpus: Path, jobs: int) -> int:
    dirs = sorted(str(d) for d in corpus.iterdir() if (d / "graph.json").is_file())
    if not dirs:
        print(f"no graphs under {corpus}")
        return 2
    t0 = time.monotonic()
    with ProcessPoolExecutor(max_workers=jobs, initializer=os.nice, initargs=(0,)) as ex:
        summaries = list(ex.map(one, dirs, chunksize=4))
    kernels = sum(s["kernels"] for s in summaries)
    dis = [(s["name"], x) for s in summaries for x in s["disagreements"]]
    errs = [(s["name"], x) for s in summaries for x in s["errors"]]
    print(
        f"oracle-ref: {len(dirs)} graphs, {kernels} kernel results, {len(dis)} disagreements between libraries, "
        f"{len(errs)} errors, {time.monotonic() - t0:.1f} s, {jobs} processes"
    )
    for name, x in dis[:40]:
        print(f"  disagree {name} {x['kernel']} {x['libraries']}: {x['detail']}")
    for name, x in errs[:20]:
        print(f"  error {name} {x['kernel']}: {x['error'].strip().splitlines()[-1]}")
    return 0 if not errs else 1
