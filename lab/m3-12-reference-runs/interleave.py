# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Exploratory, interleaved rounds of bench/specs/references.toml (#55).

    python3 interleave.py --rounds N --out rows.jsonl --work DIR [--only SUBSTRING ...]
        [--skip SUBSTRING ...] [--first-round K]

The harness runs a spec case after case; on a shared machine that turns background load
into differences between tools. This script runs the spec's own commands (as
`everygraph-bench check references --commands` expands them) in rounds: in each round
every comparison set (kernel, graph, threads) comes in a random order, and inside a set
every tool in a random order, so that each tool's time can be divided by Everygraph's
time from the same round. Each command runs as the harness runs it:

    systemd-run --user --scope -p MemoryMax=<case limit> -p MemorySwapMax=0
        -p OOMPolicy=continue -- everygraph-bench measure --report FILE -- <command>

and the row keeps the tool's metrics (the spec's JSON pointers and regular expressions)
and the cgroup's memory peaks. The spec's preparation steps must have run
(`everygraph-bench run references --prepare-only`). Not for the record: the rows say
`exploratory`, and summarize.py prints ratios, not absolute claims.
"""

import argparse
import datetime
import json
import os
import random
import re
import subprocess
import sys
import tomllib

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
BENCH = os.path.join(REPO, "target", "release", "everygraph-bench")
SPEC = os.path.join(REPO, "bench", "specs", "references.toml")
CASE = re.compile(r"^  (\S+)\s+.*limit ([0-9.]+) GiB")


def cases(data, work):
    out = subprocess.run([BENCH, "check", "references", "--commands", "--data", data, "--work", work],
                         check=True, capture_output=True, text=True).stdout.splitlines()
    found = []
    for line, command in zip(out, out[1:]):
        m = CASE.match(line)
        if m:
            command = command.strip()
            argv = ["sh", "-c", command[len("sh -c "):]] if command.startswith("sh -c ") else command.split()
            found.append({"id": m.group(1), "limit_gib": float(m.group(2)), "argv": argv})
    return found


def key(case):
    """(kernel, graph, threads): the cases whose times are divided by each other."""
    parts = case["id"].split("/")
    bench, fields, threads = parts[0], dict(p.split("=", 1) for p in parts[1:-1]), parts[-1]
    kernel, tool = bench.split("-", 1)
    graph = fields.get("graph")
    if kernel == "friendster":
        kernel, graph = fields["kernel"].removesuffix("64"), "friendster"
    return (kernel, graph, threads), tool + "".join(f" {v}" for k, v in fields.items() if k not in ("graph", "kernel"))


def metrics(bench, stdout):
    found = {}
    lines = [line for line in stdout.splitlines() if line.strip()]
    last = None
    if lines:
        try:
            last = json.loads(lines[-1])
        except json.JSONDecodeError:
            pass
    for m in bench.get("metric", []):
        value = None
        if "json" in m and last is not None:
            value = last
            for part in m["json"].strip("/").split("/"):
                value = value.get(part) if isinstance(value, dict) else None
        elif "regex" in m:
            hit = re.search(m["regex"], stdout)
            value = float(hit.group(1)) if hit else None
        if isinstance(value, (int, float)):
            value *= m.get("scale", 1)
        found[m["name"]] = value
    if isinstance(last, dict):
        for k in ("hash", "algorithm", "version"):
            if k in last:
                found[k] = last[k]
    return found


def run(case, bench, report):
    wrapper = ["systemd-run", "--user", "--scope", "-q", f"-p", f"MemoryMax={case['limit_gib']}G",
               "-p", "MemorySwapMax=0", "-p", "OOMPolicy=continue", "--",
               BENCH, "measure", "--report", report, "--timeout-ms", str(45 * 60 * 1000), "--"]
    p = subprocess.run(wrapper + case["argv"], capture_output=True, text=True)
    m = json.load(open(report)) if os.path.exists(report) else {}
    return {
        "exit": p.returncode,
        "stderr_tail": p.stderr[-400:] if p.returncode else "",
        "metrics": metrics(bench, p.stdout),
        "wall_seconds": m.get("wall_seconds"),
        "memory_peak_bytes": m.get("memory_peak_bytes"),
        "anon_peak_bytes": m.get("anon_peak_bytes"),
        "file_peak_bytes": m.get("file_peak_bytes"),
        "oom_kill": (m.get("memory_events") or {}).get("oom_kill"),
    }


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--rounds", type=int, default=3)
    p.add_argument("--out", required=True)
    p.add_argument("--only", action="append", default=[], help="keep cases whose id contains this")
    p.add_argument("--skip", action="append", default=[], help="drop cases whose id contains this")
    p.add_argument("--first-round", type=int, default=0, help="number of the first round (to add rounds)")
    p.add_argument("--data", default=os.environ.get("EVERYGRAPH_DATA", "/home/tux/graphlab/data"))
    p.add_argument("--work", required=True, help="the harness work directory the preparation used")
    p.add_argument("--seed", type=int, default=1)
    args = p.parse_args()
    spec = {b["name"]: b for b in tomllib.load(open(SPEC, "rb"))["benchmark"]}
    sets = {}
    for c in cases(args.data, args.work):
        if args.only and not any(o in c["id"] for o in args.only) or any(o in c["id"] for o in args.skip):
            continue
        k, tool = key(c)
        sets.setdefault(k, []).append((tool, c))
    rng = random.Random(args.seed + args.first_round)
    report = args.out + ".measure.json"
    revision = subprocess.run(["git", "-C", REPO, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    with open(args.out, "a") as out:
        for r in range(args.first_round, args.first_round + args.rounds):
            order = list(sets)
            rng.shuffle(order)
            for k in order:
                tools = list(sets[k])
                rng.shuffle(tools)
                for tool, c in tools:
                    load = os.getloadavg()[0]
                    row = {"exploratory": True, "round": r, "kernel": k[0], "graph": k[1], "threads": k[2],
                           "tool": tool, "case": c["id"], "revision": revision, "load_start": load,
                           "started_at": datetime.datetime.now().isoformat(timespec="seconds"),
                           **run(c, spec[c["id"].split("/")[0]], report)}
                    out.write(json.dumps(row) + "\n")
                    out.flush()
                    print(f"round {r} {c['id']}: {row['metrics'].get('seconds')} s, exit {row['exit']}, "
                          f"load {load:.1f}", file=sys.stderr, flush=True)
    if os.path.exists(report):
        os.remove(report)


if __name__ == "__main__":
    main()
