# SPDX-License-Identifier: Apache-2.0 OR MIT
"""oracle-ref: the Python side of the Everygraph correctness oracle.

  oracle-ref run CORPUS [--jobs N]          reference.json for every graph of a corpus
  oracle-ref golden (--fetch|--write|--check) [--cache DIR]
                                            golden files of small real graphs
  oracle-ref probe (--write|--check)        where NetworkX and igraph disagree
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

ORACLE = Path(__file__).resolve().parents[2]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="oracle-ref", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="compute references for a corpus directory")
    r.add_argument("corpus", type=Path)
    r.add_argument("--jobs", type=int, default=8)
    g = sub.add_parser("golden", help="golden files of small real graphs")
    mode = g.add_mutually_exclusive_group(required=True)
    mode.add_argument("--fetch", action="store_true", help="download the sources into the cache, verifying checksums")
    mode.add_argument("--write", action="store_true", help="recompute and rewrite oracle/golden/*.json")
    mode.add_argument("--check", action="store_true", help="recompute and compare with oracle/golden/*.json")
    g.add_argument("--cache", type=Path, default=None)
    p = sub.add_parser("probe", help="semantics probe: NetworkX vs igraph on tiny graphs")
    pm = p.add_mutually_exclusive_group(required=True)
    pm.add_argument("--write", action="store_true")
    pm.add_argument("--check", action="store_true")
    a = ap.parse_args(argv)
    if a.cmd == "run":
        from .run import run

        return run(a.corpus, a.jobs)
    if a.cmd == "golden":
        from . import golden

        cache = a.cache or golden.default_cache()
        if a.fetch:
            return golden.fetch(cache)
        return golden.build(cache, write=a.write)
    if a.cmd == "probe":
        from . import probe

        return probe.main(write=a.write)
    return 2


if __name__ == "__main__":
    sys.exit(main())
