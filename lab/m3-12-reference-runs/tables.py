# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The README's tables, from results/: the validation of every tool's output against
Everygraph's (`validation-*.json`, lines of `bench/runners/run.py validate`) and the
memory peaks of one harness run of each case (`once.jsonl`, harness rows; the table
shows orkut at 16 threads and Friendster). Neither
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
    cells, facts = defaultdict(dict), defaultdict(dict)
    for path in sorted(glob.glob(os.path.join(HERE, "validation-*.json"))):
        if path.endswith("-first-lj.json"):  # the first check, with the tools of an earlier build
            continue
        line = json.loads(open(path).read().splitlines()[-1])
        for r in line["results"]:
            cells[(r["kernel"], r["tool"])][line["graph"]] = "yes" if r["valid"] else f"**no** ({r['detail']})"
            if r["valid"]:
                facts[r["kernel"]][line["graph"]] = r["detail"].split(" (ours")[0]
    graphs = [g for g in GRAPHS if any(g in c for c in cells.values())]
    print("| kernel | tool | " + " | ".join(graphs) + " |")
    print("|---|---|" + "---|" * len(graphs))
    for (kernel, tool), by in sorted(cells.items()):
        print(f"| {kernel} | {tool} | " + " | ".join(by.get(g, "—") for g in graphs) + " |")
    print("\n| kernel | " + " | ".join(graphs) + " |")
    print("|---|" + "---|" * len(graphs))
    for kernel in ("cc", "kcore", "bfs", "sssp", "tc"):
        row = facts[kernel]
        if kernel == "tc":
            row = {g: d.split(": ")[-1].split(",")[0].rstrip("}") + " triangles" for g, d in row.items()}
        print(f"| {kernel} | " + " | ".join(row.get(g, "—") for g in graphs) + " |")


def memory():
    path = os.path.join(HERE, "once.jsonl")
    if not os.path.exists(path):
        return
    rows = [json.loads(line) for line in open(path)]
    print("\n| case | status | anon peak GiB | memory.peak GiB | file peak GiB |")
    print("|---|---|---|---|---|")
    for r in rows:
        if "graph=orkut/t16" not in r["case"] and "type=" not in r["case"] and not r["case"].startswith("friendster"):
            continue
        m = r.get("memory") or {}
        f = lambda k: f"{m[k] / GIB:.2f}" if m.get(k) is not None else "—"
        print(f"| {r['case']} | {r['status']} | {f('anon_peak_bytes')} | {f('peak_bytes')} | {f('file_peak_bytes')} |")


if __name__ == "__main__":
    validation()
    memory()
