#!/usr/bin/env python3
"""Companion file of an official ooc-leiden run: for every harness row, the program's own last
JSON line (per-pass phases, bytes read per phase, accountant report, hashes), taken from the
harness's log of that repetition ({work}/ooc-leiden/logs/<case>/rep<N>.out).

usage: collect_program.py ROWS.jsonl LOGS_DIR  ->  writes ROWS.program.jsonl next to ROWS.jsonl
"""
import json
import os
import sys

rows_f, logs = sys.argv[1], sys.argv[2]
out = rows_f[: -len(".jsonl")] + ".program.jsonl"
n = 0
with open(out, "w") as o:
    for l in open(rows_f):
        r = json.loads(l)
        d = r["case"].replace("/", "__").replace("=", "-")
        p = os.path.join(logs, d, f"rep{r['repetition']}.out")
        js = [x for x in open(p) if x.startswith("{")]
        prog = json.loads(js[-1])
        rec = {"run_id": r["run_id"], "case": r["case"], "repetition": r["repetition"], "log": f"{{work}}/ooc-leiden/logs/{d}/rep{r['repetition']}.out"}
        rec.update(prog)
        o.write(json.dumps(rec, separators=(",", ":")) + "\n")
        n += 1
print(n, "lines ->", out)
