# SPDX-License-Identifier: Apache-2.0 OR MIT
"""oracle-ref: the Python side of the Everygraph correctness oracle.

  oracle-ref run CORPUS [--jobs N] [--max-memory-gib G] [--max-rows R]
                                            reference.json for every graph of a corpus
  oracle-ref golden (--fetch|--write|--check) [--cache DIR] [--data-root DIR]
                                            golden files of small real graphs
  oracle-ref probe (--write|--check)        where NetworkX and igraph disagree
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

ORACLE = Path(__file__).resolve().parents[2]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="oracle-ref", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="compute references for a corpus directory")
    r.add_argument("corpus", type=Path)
    r.add_argument("--jobs", type=int, default=2, help="worker processes (default 2)")
    r.add_argument("--max-memory-gib", type=float, default=3.0, help="address-space limit per worker (default 3 GiB)")
    r.add_argument("--max-rows", type=int, default=2_000_000, help="graphs with more rows are skipped (default 2,000,000)")
    g = sub.add_parser("golden", help="golden files of small real graphs")
    mode = g.add_mutually_exclusive_group(required=True)
    mode.add_argument("--fetch", action="store_true", help="download the sources into the cache, verifying checksums")
    mode.add_argument("--write", action="store_true", help="recompute and rewrite oracle/golden/*.json")
    mode.add_argument("--check", action="store_true", help="recompute and compare with oracle/golden/*.json")
    g.add_argument("--cache", type=Path, default=None, help="downloads of the non-registry graphs (default oracle/.cache)")
    g.add_argument("--data-root", type=Path, default=None, help="data root of `everygraph-bench fetch tier1` (default $EVERYGRAPH_ORACLE_DATA, then $EVERYGRAPH_DATA)")
    g.add_argument("--max-memory-gib", type=float, default=6.0, help="address-space limit of the process (default 6 GiB)")
    g.add_argument("--only", nargs="*", default=None, help="only these graphs")
    p = sub.add_parser("probe", help="semantics probe: NetworkX vs igraph on tiny graphs")
    pm = p.add_mutually_exclusive_group(required=True)
    pm.add_argument("--write", action="store_true")
    pm.add_argument("--check", action="store_true")
    a = ap.parse_args(argv)
    if a.cmd == "run":
        # One BLAS thread per worker: the parallelism is the worker count, and a BLAS
        # pool per worker would multiply both cores and reserved memory.
        for var in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
            os.environ.setdefault(var, "1")
        from .run import run

        return run(a.corpus, a.jobs, a.max_memory_gib, a.max_rows)
    if a.cmd == "golden":
        from . import golden

        for var in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
            os.environ.setdefault(var, "1")
        cache = a.cache or golden.default_cache()
        if a.fetch:
            return golden.fetch(cache)
        return golden.build(cache, a.data_root or golden.default_data_root(), write=a.write, memory_gib=a.max_memory_gib, only=a.only)
    if a.cmd == "probe":
        from . import probe

        return probe.main(write=a.write)
    return 2


if __name__ == "__main__":
    sys.exit(main())
