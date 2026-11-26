# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Tables of lab/m3-27-frontier-counters/README.md from results/: run as
`python3 -I summarize.py results` and it prints Markdown and writes results/summary.json.

Per frontier call (a k-core sub-round or exact count, an SSSP step) the call log holds
the entries handed to bodies (side lists included), the runs of 256 nodes, the largest
run, the longest list, the longest side chain of one body call, and the draws.

The span model (an estimate, not a measurement) counts entries on the critical path of
each call at P = 16 workers, then sums over calls:
- now: a call of one run is done by one thread: E. Otherwise max(E / P, largest run).
- S3: runs cut by entries and lists split at L = 1,024 entries: max(E / P, min(E, L),
  longest side chain), the side chain (a local queue) being work one task must do.
- ideal: E / P.
"""

import gzip
import json
import sys
from collections import defaultdict
from pathlib import Path

P = 16
L = 1024
CLASSES = [(0, 1 << 10), (1 << 10, 1 << 16), (1 << 16, 1 << 20), (1 << 20, 1 << 62)]


def class_name(lo, hi):
    names = {0: "0", 1 << 10: "1Ki", 1 << 16: "64Ki", 1 << 20: "1Mi"}
    return f"[{names[lo]}, {names.get(hi, 'inf')})"


# k-core's exact counts split each hub list themselves (`par::map_ranges`, 16,384
# entries per part): their span is about E / P whatever the runs.
NESTED = {"kcore.exact": 1 << 14}


def span_now(tag, c):
    if tag in NESTED:
        return max(c["entries"] / P, min(c["entries"], NESTED[tag]))
    return c["entries"] if c["runs"] <= 1 else max(c["entries"] / P, c["max_run"])


def span_s3(tag, c):
    if tag in NESTED:
        return span_now(tag, c)
    return max(c["entries"] / P, min(c["entries"], L), c["max_chain"])


def calls_summary(calls):
    out = {}
    by_tag = defaultdict(list)
    for c in calls:
        by_tag[c["tag"]].append(c)
    for tag, cs in sorted(by_tag.items()):
        e = sum(c["entries"] for c in cs)
        single = [c for c in cs if c["runs"] <= 1]
        es = sum(c["entries"] for c in single)
        now = sum(span_now(tag, c) for c in cs)
        s3 = sum(span_s3(tag, c) for c in cs)
        ideal = sum(c["entries"] / P for c in cs)
        classes = []
        for lo, hi in CLASSES:
            k = [c for c in cs if lo <= c["entries"] < hi]
            classes.append({
                "class": class_name(lo, hi),
                "calls": len(k),
                "entries": sum(c["entries"] for c in k),
                "single_run_calls": sum(1 for c in k if c["runs"] <= 1),
                "single_run_entries": sum(c["entries"] for c in k if c["runs"] <= 1),
                "now_span": sum(span_now(tag, c) for c in k),
            })
        out[tag] = {
            "calls": len(cs),
            "keys": len({c["key"] for c in cs}),
            "nodes": sum(c["nodes"] for c in cs),
            "entries": e,
            "runs": sum(c["runs"] for c in cs),
            "single_run_calls": len(single),
            "single_run_entries": es,
            "single_run_share": es / e if e else 0.0,
            "side_lists": sum(c["side_lists"] for c in cs),
            "draws": sum(c["draws"] for c in cs),
            "max_list": max((c["max_list"] for c in cs), default=0),
            "span_now": now,
            "span_s3": s3,
            "span_ideal": ideal,
            "classes": classes,
        }
    return out


def marks_summary(run):
    steps = defaultdict(lambda: [0, 0, 0, 0])
    prev = (0, 0, 0)
    for m in run["marks"]:
        cur = (m["decoded"], m["handed"], m["examined"])
        d = [a - b for a, b in zip(cur, prev)]
        s = steps[m["tag"]]
        s[0] += 1
        for i in range(3):
            s[i + 1] += d[i]
        prev = cur
    return {t: {"steps": s[0], "decoded": s[1], "handed": s[2], "examined": s[3]} for t, s in steps.items()}


def main(res):
    res = Path(res)
    runs = [json.loads(line) for line in (res / "runs.jsonl").read_text().splitlines() if line]
    summary = []
    for r in runs:
        name = f"{r['graph']}-{r['kernel']}-t{r['threads']}-{r['run']}"
        row = {k: r[k] for k in ("run", "graph", "kernel", "source", "threads", "nodes", "entries", "hash", "restarts")}
        row["entries_in_lists_ge"] = r["entries_in_lists_ge"]
        log = res / "calls" / f"{name}.jsonl.gz"
        if log.exists():
            with gzip.open(log, "rt") as f:
                row["frontier"] = calls_summary([json.loads(x) for x in f if x.strip()])
        if r["marks"]:
            row["steps"] = marks_summary(r)
            if r["kernel"] == "wcc":
                row["wcc_sample_touched"] = r["nodes_deg_ge1"] + r["nodes_deg_ge2"]
        summary.append(row)
    (res / "summary.json").write_text(json.dumps(summary, indent=1) + "\n")
    report(summary)


def pct(x):
    return f"{100 * x:.1f}%"


def big(x):
    return f"{x:,.0f}"


def report(summary):
    print("## Frontier calls\n")
    print("| graph | kernel | run | tag | calls | rounds/keys | entries | runs | single-run calls | their entries | share | draws | restarts | longest list | span now / S3 / ideal (M entries) | now/S3 |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for r in summary:
        for tag, s in r.get("frontier", {}).items():
            print(f"| {r['graph']} | {r['kernel']} | t{r['threads']}{r['run']} | {tag} | {big(s['calls'])} | {big(s['keys'])} | {big(s['entries'])} | {big(s['runs'])} | "
                  f"{big(s['single_run_calls'])} | {big(s['single_run_entries'])} | {pct(s['single_run_share'])} | {big(s['draws'])} | {r['restarts']} | {big(s['max_list'])} | "
                  f"{s['span_now'] / 1e6:.1f} / {s['span_s3'] / 1e6:.1f} / {s['span_ideal'] / 1e6:.1f} | {s['span_now'] / max(s['span_s3'], 1):.2f} |")
    print("\n## Calls by entries\n")
    print("| graph | kernel | run | tag | class | calls | entries | single-run calls | their entries | span now (M) |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for r in summary:
        for tag, s in r.get("frontier", {}).items():
            for c in s["classes"]:
                print(f"| {r['graph']} | {r['kernel']} | t{r['threads']}{r['run']} | {tag} | {c['class']} | {big(c['calls'])} | {big(c['entries'])} | "
                      f"{big(c['single_run_calls'])} | {big(c['single_run_entries'])} | {c['now_span'] / 1e6:.1f} |")
    print("\n## Decode surplus\n")
    print("| graph | kernel | step | steps | decoded | handed | examined | decoded/examined | decoded/handed | handed/examined |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for r in summary:
        st = r.get("steps")
        if not st:
            continue
        tot = [0, 0, 0]
        for tag, s in st.items():
            tot = [tot[0] + s["decoded"], tot[1] + s["handed"], tot[2] + s["examined"]]
            q = lambda a, b: f"{a / b:.2f}" if b else "-"
            print(f"| {r['graph']} | {r['kernel']} | {tag} | {s['steps']} | {big(s['decoded'])} | {big(s['handed'])} | {big(s['examined'])} | "
                  f"{q(s['decoded'], s['examined'])} | {q(s['decoded'], s['handed'])} | {q(s['handed'], s['examined'])} |")
        q = lambda a, b: f"{a / b:.2f}" if b else "-"
        print(f"| {r['graph']} | {r['kernel']} | **all** | | {big(tot[0])} | {big(tot[1])} | {big(tot[2])} | {q(tot[0], tot[2])} | {q(tot[0], tot[1])} | {q(tot[1], tot[2])} |")
        if "wcc_sample_touched" in r:
            print(f"| {r['graph']} | {r['kernel']} | sample, entries used (1 or 2 per list) | | | | {big(r['wcc_sample_touched'])} | | | |")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "results")
