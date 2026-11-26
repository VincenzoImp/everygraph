#!/usr/bin/env python3
"""Markdown tables for the README from summarize.py's JSON.   python3 tables.py results/summary-lj-orkut.json"""
import json
import sys

d = json.load(open(sys.argv[1]))
T = {(e["graph"], e["iterations"], e["variant"], e["threads"]): e for e in d["table"]}
S = {(e["graph"], e["iterations"], e["variant"]): e for e in d.get("seeds", [])}
V = ["auto", "s4", "s8", "s16", "s32", "s64", "L16k", "L64k", "L64k-max16"]
cols = [("lj", 1), ("lj", 3), ("orkut", 1), ("orkut", 3)]
for t in (16, 4):
    print(f"\ndet/async time at {t} threads (in brackets: time / auto's), median of in-block ratios over rounds\n")
    print("| rule | " + " | ".join(f"{g}, {i} it." for g, i in cols) + " |")
    print("|---|" + "---|" * len(cols))
    for v in V:
        print(f"| {v} | " + " | ".join(f"{T[(g, i, v, t)]['ratio_async']:.2f} ({T[(g, i, v, t)]['ratio_auto']:.2f})" for g, i in cols) + " |")
print("\nload-independent: work (scanned entries / level-0 entries), iterations, levels at the 20-iteration cap,"
      " sub-rounds run by all threads / by one thread, modularity at the default seed and over seeds\n")
for g, i in cols:
    print(f"\n{g}, {i} iteration(s)\n")
    print("| rule | work | iterations | capped levels | sub-rounds all / one thread | Q (seed 4) | Q over seeds: mean [min, max] (n) | communities | disconnected |")
    print("|---|---|---|---|---|---|---|---|---|")
    for v in V + ["async"]:
        e = T[(g, i, v, 16)]
        s = S.get((g, i, v))
        sq = f"{s['mean']:.4f} [{s['min']:.4f}, {s['max']:.4f}] ({s['n']})" if s else ""
        sr = f"{e['subrounds_par']} / {e['subrounds_seq']}" if e["subrounds_par"] is not None else "—"
        q = f"{e['modularity']:.4f}" if v != "async" else f"{min(e['modularity_values']):.4f}–{max(e['modularity_values']):.4f}"
        print(f"| {v} | {e['work_edge_passes']:.1f} | {e['total_iters']} | {e['passes_at_iter_cap']} | {sr} | {q} | {sq} | {e['communities']} | {e['disconnected']} |")
