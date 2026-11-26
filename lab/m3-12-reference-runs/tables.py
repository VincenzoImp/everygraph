# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The README's tables, from results/: the validation of every tool's output against
Everygraph's (`validation-*.json`, lines of `bench/runners/run.py validate`) and the
memory peaks of one harness run of each case (`once.jsonl`, harness rows). Neither
depends on the machine's load; no time is printed.

    python3 tables.py
"""

import glob
import json
import os
from collections import defaultdict

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "results")
GRAPHS = ["lj", "orkut", "roadnet-ca", "web-google", "twitter-sym", "friendster"]
GIB = 1 << 30


def validation():
    cells = defaultdict(dict)
    for path in glob.glob(os.path.join(HERE, "validation-*.json")):
        line = json.loads(open(path).read().splitlines()[-1])
        for r in line["results"]:
            mark = "yes" if r["valid"] else "**no**"
            cells[(r["kernel"], r["tool"])][line["graph"]] = f"{mark}: {r['detail']}"
        for s in line["skipped"]:
            print(f"<!-- {line['graph']}: skipped {s} -->")
    graphs = [g for g in GRAPHS if any(g in c for c in cells.values())]
    print("| kernel | tool | " + " | ".join(graphs) + " |")
    print("|---|---|" + "---|" * len(graphs))
    for (kernel, tool), by in sorted(cells.items()):
        print(f"| {kernel} | {tool} | " + " | ".join(by.get(g, "—") for g in graphs) + " |")


def memory():
    path = os.path.join(HERE, "once.jsonl")
    if not os.path.exists(path):
        return
    rows = [json.loads(line) for line in open(path)]
    print("\n| case | status | anon peak GiB | memory.peak GiB | file peak GiB |")
    print("|---|---|---|---|---|")
    for r in rows:
        m = r.get("memory") or {}
        f = lambda k: f"{m[k] / GIB:.2f}" if m.get(k) is not None else "—"
        print(f"| {r['case']} | {r['status']} | {f('anon_peak_bytes')} | {f('peak_bytes')} | {f('file_peak_bytes')} |")


if __name__ == "__main__":
    validation()
    memory()
