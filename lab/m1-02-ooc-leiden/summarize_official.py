#!/usr/bin/env python3
"""Tables of the official ooc-leiden run (issue #16) from the harness rows and the program's own
JSON lines (bench/results/ooc-leiden/<run>.jsonl and <run>.program.jsonl).

Per case: counted repetitions, median of each phase (seconds), modularity, communities,
disconnected, the harness's anonymous peak, the accountant's peak, bytes read per phase (the
program's /proc/diskstats deltas: machine-wide, valid on the idle machine), the harness's
read_bytes, and the ratio of the kernel time to the in-RAM reference of the same group.

usage: summarize_official.py ROWS.jsonl PROGRAM.jsonl
"""
import json
import statistics as st
import sys
from collections import defaultdict

rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
prog = defaultdict(list)
for l in open(sys.argv[2]):
    if l.strip():
        d = json.loads(l)
        prog[d["case"]].append(d)

GiB = 2**30
cases = defaultdict(list)
for r in rows:
    if r.get("status") == "ok" and not (r.get("disturbance") or {}).get("disturbed"):
        cases[r["case"]].append(r)
others = [(r["case"], r.get("repetition"), r.get("status"), (r.get("disturbance") or {}).get("reasons")) for r in rows
          if r.get("status") != "ok" or (r.get("disturbance") or {}).get("disturbed")]

def med(v):
    v = [x for x in v if x is not None]
    return st.median(v) if v else None

def m(rs, k):
    return med([r["metrics"].get(k) for r in rs])

ref = {}
for c, rs in cases.items():
    if c.startswith("inram-friendster"):
        ref["friendster"] = m(rs, "seconds")
    if c.startswith("inram-orkut"):
        ref["orkut"] = m(rs, "seconds")
    if c.startswith("l1-inram-friendster"):
        ref["l1"] = m(rs, "seconds")
    if c.startswith("det-inram-friendster"):
        ref["det"] = m(rs, "seconds")

def refof(c):
    if c.startswith("l1-"):
        return ref.get("l1")
    if c.startswith("det-"):
        return ref.get("det")
    if "orkut" in c:
        return ref.get("orkut")
    return ref.get("friendster")

hdr = ["case", "n", "s", "L0 lm", "L0 ref", "L0 agg", "upper", "Q (min-max)", "comms", "disc", "anon peak GiB", "acct peak GiB",
       "read L0 lm+ref GB", "read L0 agg GB", "write L0 agg GB", "read all GB (harness)", "ratio"]
print("| " + " | ".join(hdr) + " |")
print("|" + "---|" * len(hdr))
for c in sorted(cases):
    rs = cases[c]
    ps = [p for p in prog.get(c, []) if p.get("status") == "ok"]
    def pm(f):
        return med([f(p) for p in ps])
    q = [r["metrics"].get("modularity") for r in rs]
    s = m(rs, "seconds")
    rf = refof(c)
    def f(x, nd=1):
        return "–" if x is None else f"{x:.{nd}f}"
    cells = [
        f"`{c}`", str(len(rs)), f(s), f(pm(lambda p: p["level0_lm_s"])), f(pm(lambda p: p["level0_refine_s"])),
        f(pm(lambda p: p["level0_agg_s"])), f(pm(lambda p: p["upper_levels_s"])),
        f"{st.median(q):.4f} ({min(q):.4f}-{max(q):.4f})", f(m(rs, "communities"), 0), f(m(rs, "disconnected"), 0),
        f(med([r["memory"]["anon_peak_bytes"] for r in rs]) / GiB, 2),
        f((pm(lambda p: p["accountant"]["peak_bytes"]) or 0) / GiB, 2),
        f((pm(lambda p: p["passes"][0]["lm_ref_read_bytes"]) or 0) / 1e9),
        f((pm(lambda p: p["passes"][0]["agg_read_bytes"]) or 0) / 1e9),
        f((pm(lambda p: p["passes"][0]["agg_write_bytes"]) or 0) / 1e9),
        f(med([r["io"]["read_bytes"] for r in rs]) / 1e9),
        "–" if (rf is None or s is None) else f"{s / rf:.2f}",
    ]
    print("| " + " | ".join(cells) + " |")
if others:
    print("\nnot counted:")
    for o in others:
        print("  ", o)
for c in sorted(prog):
    hs = sorted({p.get("hash_levels") for p in prog[c] if p.get("det")})
    if hs:
        print(f"hash {c}: {hs}")
