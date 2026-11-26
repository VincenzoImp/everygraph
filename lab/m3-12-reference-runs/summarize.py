# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Tables of interleave.py's rows: per kernel, graph and thread count, each tool's time
divided by Everygraph's time from the same round (above 1: Everygraph is faster), its
median kernel time, and its memory peaks.

    python3 summarize.py results/rounds.jsonl [more.jsonl ...] > tables.md

Communities divide by the lab's Leiden (`lab`); triangles have no Everygraph kernel and
show times only. Also writes results/summary.json next to the first file.
"""

import json
import os
import statistics
import sys
from collections import defaultdict

BASE = {"leiden": "lab"}
GIB = 1 << 30
ORDER = ["pagerank", "cc", "kcore", "bfs", "sssp", "tc", "leiden"]
TITLES = {"pagerank": "PageRank (f64, 20 iterations)", "cc": "Connected components", "kcore": "k-core (core numbers)",
          "bfs": "BFS (mean per source, GAP's 64 sources)", "sssp": "SSSP (mean per source, GAP's 64 sources)",
          "tc": "Triangle count (no Everygraph kernel yet)", "leiden": "Communities (Leiden, Louvain)"}


def median(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def fmt(x, digits=3):
    if x is None:
        return "—"
    return f"{x:.{digits}g}" if abs(x) < 1000 else f"{x:.0f}"


def main():
    rows = [json.loads(line) for path in sys.argv[1:] for line in open(path)]
    by = defaultdict(lambda: defaultdict(dict))  # (kernel, graph, threads) -> tool -> round -> row
    for r in rows:
        if r["exit"] == 0 and r["metrics"].get("seconds") is not None:
            by[(r["kernel"], r["graph"], r["threads"])][r["tool"]][r["round"]] = r
    summary = []
    for kernel in ORDER:
        keys = sorted((k for k in by if k[0] == kernel), key=lambda k: (k[1], int(k[2][1:])))
        if not keys:
            continue
        base = BASE.get(kernel, "eg")
        quality = kernel == "leiden"
        print(f"\n### {TITLES[kernel]}\n")
        head = "| graph | threads | tool | ratio to " + ("lab Leiden" if quality else "Everygraph") + \
               " (median, min–max) | kernel s (median) | load s | anon peak GiB | memory.peak GiB |"
        if quality:
            head += " modularity | communities |"
        print(head)
        print("|" + "---|" * (head.count("|") - 1))
        for k in keys:
            tools = by[k]
            ref = tools.get(base, {})
            names = sorted(tools, key=lambda t: (t != base, t))
            for t in names:
                runs = tools[t]
                ratios = [runs[r]["metrics"]["seconds"] / ref[r]["metrics"]["seconds"]
                          for r in runs if r in ref and ref[r]["metrics"]["seconds"]]
                secs = median([x["metrics"]["seconds"] for x in runs.values()])
                load = median([x["metrics"].get("load_seconds") for x in runs.values()])
                anon = median([x["anon_peak_bytes"] for x in runs.values()])
                peak = median([x["memory_peak_bytes"] for x in runs.values()])
                if t == base:
                    ratio = "1 (base)" if ref else "—"
                elif ratios:
                    ratio = f"{fmt(median(ratios))} ({fmt(min(ratios))}–{fmt(max(ratios))}), {len(ratios)} rounds"
                else:
                    ratio = "—"
                line = f"| {k[1]} | {k[2][1:]} | {t} | {ratio} | {fmt(secs)} | {fmt(load)} | " \
                       f"{fmt(anon / GIB if anon else None)} | {fmt(peak / GIB if peak else None)} |"
                entry = {"kernel": kernel, "graph": k[1], "threads": int(k[2][1:]), "tool": t, "base": base,
                         "ratio_median": median(ratios) if t != base else 1.0,
                         "ratio_min": min(ratios) if ratios else None, "ratio_max": max(ratios) if ratios else None,
                         "rounds": len(runs), "seconds_median": secs, "load_seconds_median": load,
                         "anon_peak_bytes_median": anon, "memory_peak_bytes_median": peak,
                         "hashes": sorted({x["metrics"].get("hash") for x in runs.values() if x["metrics"].get("hash")})}
                if quality:
                    q = median([x["metrics"].get("modularity") for x in runs.values()])
                    c = median([x["metrics"].get("communities") for x in runs.values()])
                    line += f" {fmt(q, 4)} | {fmt(c)} |"
                    entry.update(modularity_median=q, communities_median=c)
                print(line)
                summary.append(entry)
    failed = [r for r in rows if r["exit"] != 0]
    if failed:
        print("\nFailed runs:\n")
        for r in failed:
            print(f"- round {r['round']} {r['case']}: exit {r['exit']}, oom_kill {r.get('oom_kill')}")
    out = os.path.join(os.path.dirname(os.path.abspath(sys.argv[1])), "summary.json")
    with open(out, "w") as f:
        json.dump({"exploratory": True, "rows": len(rows), "cases": summary}, f, indent=1)


if __name__ == "__main__":
    main()
