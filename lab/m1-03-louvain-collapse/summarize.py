#!/usr/bin/env python3
"""Summarize m1-03 JSON lines: one line per run, then groups (mean, min, max of Q).

    python3 summarize.py results/*.jsonl [--group config,build,threads,mem,flags]

Columns: Q (b2's evaluation on the input graph), communities, largest community in input
vertices, iterations per pass, passes ending at the iteration cap, coarse moves (passes >= 1,
or every pass of a replay), back moves (a vertex returning to the community it left within
two iterations), and race loss = sum of claimed gains - sum of true modularity changes over
the traced iterations (0 up to rounding at one thread).
"""
import argparse
import json
import statistics as st
from collections import defaultdict


def row_stats(r):
    b = r["b2"]
    passes = b["passes"]
    cap = b.get("max_iters", 20)
    traced = [t for p in passes for t in p.get("trace", [])]
    loss = sum(t["claimed"] - (t["q"] - t["q_before"]) for t in traced if "q_before" in t)
    return {
        "Q": b["modularity"],
        "comms": b["communities"],
        "largest": b.get("largest_nodes", b["largest"]),
        "iters": [p["iters"] for p in passes],
        "capped": sum(1 for p in passes if p["iters"] >= cap),
        "coarse_moves": sum(t["moves"] for t in traced),
        "back": sum(t["back"] for t in traced),
        "loss": loss,
        "top_share": max((t["top_share"] for t in traced), default=float("nan")),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--group", default="config,build,threads,mem,flags")
    ap.add_argument("--rows", action="store_true")
    a = ap.parse_args()
    rows = [json.loads(l) for f in a.files for l in open(f) if l.strip()]
    keys = a.group.split(",")
    groups = defaultdict(list)
    for r in rows:
        if "b2" not in r:
            print(f"FAILED {r['tag']} rc {r['rc']}")
            continue
        r.setdefault("threads", r["b2"]["threads"])
        s = row_stats(r)
        if a.rows:
            print(f"{r['tag']:48s} Q {s['Q']:.4f} comms {s['comms']:7d} largest {s['largest']:9d} iters {s['iters']} "
                  f"capped {s['capped']} moves {s['coarse_moves']} back {s['back']} loss {s['loss']:.4f} top {s['top_share']:.3f}")
        groups[tuple(str(r.get(k, "")) for k in keys)].append(s)
    print()
    print(" | ".join(keys) + " | n | Q mean [min, max] | largest [min, max] | capped passes (mean) | back moves (mean) | race loss (mean)")
    for k, ss in sorted(groups.items()):
        q = [s["Q"] for s in ss]
        lg = [s["largest"] for s in ss]
        print(" | ".join(k) + f" | {len(ss)} | {st.mean(q):.4f} [{min(q):.4f}, {max(q):.4f}] | "
              f"[{min(lg)}, {max(lg)}] | {st.mean(s['capped'] for s in ss):.1f} | "
              f"{st.mean(s['back'] for s in ss):.0f} | {st.mean(s['loss'] for s in ss):.4f}")


if __name__ == "__main__":
    main()
