# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The tables of bench/specs/references.toml, from its rows and its validation:

    python3 tables.py DIR ROWS.jsonl [ROWS.jsonl ...] [--times]

- the validation of every tool's output against Everygraph's (`validation-<graph>.json` in
  DIR, lines of `bench/runners/run.py validate`);
- memory peaks, and with `--times` (official rows only) the median kernel time of each
  case with its spread, and for Leiden its modularity with its spread.

A row is shown only when its tool passed the last validation of its graph and kernel
(`passed`): a tool whose output was not checked, or differs, is not reported (#265).
Leiden tools do not run with equal settings, so their table has no ratio.
"""

import glob
import json
import os
import statistics
import sys
from collections import defaultdict

GRAPHS = ["lj", "orkut", "roadnet-ca", "web-google", "twitter-sym", "friendster"]
GIB = 1 << 30
KERNELS = {"pagerank": "pr64", "cc": "cc", "kcore": "kcore", "bfs": "bfs", "sssp": "sssp", "tc": "tc",
           "leiden": "leiden", "louvain": "leiden"}


def load_validation(directory):
    """{graph: line of run.py validate}, from DIR/validation-<graph>.json."""
    out = {}
    for path in sorted(glob.glob(os.path.join(directory, "validation-*.json"))):
        line = json.loads(open(path).read().splitlines()[-1])
        out[line["graph"]] = line
    return out


def check_of(row):
    """(graph, kernel, tool) of the validation check that covers a harness row."""
    bench, v = row["benchmark"], row.get("vars", {})
    if bench.startswith("friendster-"):
        tool, (kernel, graph) = bench.split("-", 1)[1], (v.get("kernel", "cc"), "friendster")
        if tool == "ucr":
            tool, kernel = kernel.split("/")
        elif tool == "gbbs" and kernel == "connectit":
            tool, kernel = "connectit", "cc"
        elif tool == "cost":
            kernel = "cc"
        return graph, kernel.removesuffix("64"), tool
    family, tool = bench.split("-", 1)
    kernel = KERNELS[family]
    if kernel == "leiden":
        tool = {"lab": "lab", "lab-det": "lab-det", "gve": f"gve-{v.get('type')}", "igraph": "igraph",
                "networkit": "networkit-leiden" if family == "leiden" else "networkit-plm"}[tool]
    return v.get("graph"), kernel, tool


def passed(row, validation):
    """Whether the row's tool passed the last validation of its graph and kernel.
    Everygraph's own rows (`eg`) need a validation of the graph that skipped nothing."""
    graph, kernel, tool = check_of(row)
    line = validation.get(graph)
    if line is None or line.get("skipped_count", len(line.get("skipped", []))) > 0:
        return False
    if tool == "eg":
        return any(r["kernel"] == kernel for r in line["results"])
    return any(r["kernel"] == kernel and r["tool"] == tool and r["valid"] for r in line["results"])


def validation_table(validation):
    cells, facts = defaultdict(dict), defaultdict(dict)
    for graph, line in validation.items():
        for r in line["results"]:
            cells[(r["kernel"], r["tool"])][graph] = "yes" if r["valid"] else f"**no** ({r['detail']})"
            if r["valid"] and r["kernel"] != "leiden":
                facts[r["kernel"]][graph] = r["detail"].split(" (ours")[0]
    graphs = [g for g in GRAPHS if g in validation]
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


def spread(xs):
    xs = sorted(x for x in xs if x is not None)
    return (statistics.median(xs), xs[0], xs[-1]) if xs else (None, None, None)


def case_table(rows, validation, times):
    by_case = defaultdict(list)
    for r in rows:
        if r["status"] == "ok" and r["group"] != "validation" and passed(r, validation):
            by_case[r["case"]].append(r)
    head = "| case | anon peak GiB | memory.peak GiB |"
    if times:
        head += " kernel s, median (min–max) | modularity, median (min–max) |"
    print("\n" + head)
    print("|---|" + "---|" * (head.count("|") - 2))
    for case, rs in by_case.items():
        anon = statistics.median(r["memory"]["anon_peak_bytes"] for r in rs) / GIB
        peak = statistics.median(r["memory"]["peak_bytes"] for r in rs) / GIB
        line = f"| {case} | {anon:.2f} | {peak:.2f} |"
        if times:
            t, q = (spread(r["metrics"].get(k) for r in rs) for k in ("seconds", "modularity"))
            line += f" {t[0]:.4g} ({t[1]:.4g}–{t[2]:.4g}) |" if t[0] is not None else " — |"
            line += f" {q[0]:.4f} ({q[1]:.4f}–{q[2]:.4f}) |" if q[0] is not None else " — |"
        print(line)


def main(argv):
    times = "--times" in argv
    args = [a for a in argv if a != "--times"]
    validation = load_validation(args[0])
    rows = [json.loads(line) for path in args[1:] for line in open(path)]
    validation_table(validation)
    case_table(rows, validation, times)


if __name__ == "__main__":
    main(sys.argv[1:])
