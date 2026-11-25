#!/usr/bin/env python3
"""Summarize orders.sh rows (exploratory): per graph, mode (in RAM / out of core) and visiting
order: modularity (mean, min-max over runs), communities, time (median), and out of core the bytes
read from the disk and the L3 trace (share of 64-node blocks and of store pages touched per
level-0 local-moving iteration)."""
import json
import statistics as st
import sys
from collections import defaultdict

rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip().startswith("{")]
groups = defaultdict(list)
for d in rows:
    if d.get("status") != "ok":
        continue
    if not d["permute"]:
        order = "natural"
    elif d["order_window"]:
        order = f"win{d['order_window']}"
    else:
        order = "block"
    mode = "ooc" if d["ooc"] else "inram"
    groups[(d["graph"], mode, order)].append(d)

print(f"{'graph':13} {'mode':5} {'order':9} {'runs':>4} {'Q mean':>8} {'Q min-max':>15} {'comms':>7} {'time s':>7} {'lvl0 lm s':>9} {'read GB':>8}  L3 trace level 0: active blocks % / touched pages % per iteration")
for key in sorted(groups):
    v = groups[key]
    q = [d["modularity"] for d in v]
    t = st.median(d["time_s"] for d in v)
    lm = st.median(d["level0_lm_s"] for d in v)
    rd = st.median(d.get("read_bytes", 0) for d in v) / 1e9
    tr = ""
    trace = [x for x in v[0].get("trace", []) if x["n"] == v[0]["n"]]
    if trace:
        tr = " ".join(f"{100*x['active_blocks']/x['blocks']:.0f}/{100*x['touched_pages']/x['store_pages']:.0f}" for x in trace)
    print(f"{key[0]:13} {key[1]:5} {key[2]:9} {len(v):4} {st.mean(q):8.4f} {min(q):7.4f}-{max(q):7.4f} {round(st.mean(d['communities'] for d in v)):7} {t:7.3f} {lm:9.3f} {rd:8.2f}  {tr}")
