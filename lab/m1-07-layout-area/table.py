#!/usr/bin/env python3
"""table.py — Markdown tables from results/ of m1-07, with DRGraph and random from m1-18."""
import json, os

R = os.path.join(os.path.dirname(os.path.abspath(__file__)), "results")
B = os.path.join(os.path.dirname(R), "..", "m1-18-layout-baseline", "results")
OURS = ["hcl", "hcls", "hclsb", "cells", "cellsb"]


def f(x, d=3):
    return "—" if x is None else f"{x:.{d}f}"


def load(path):
    return json.load(open(path)) if os.path.exists(path) else None


for g in ["lj", "orkut"]:
    print(f"## {g}\n")
    M = {v: load(f"{R}/metrics/{v}_{g}.json") for v in OURS}
    M["DRGraph (m1-18)"] = load(f"{B}/metrics/drgraph_{g}.json")
    M["random (m1-18)"] = load(f"{B}/metrics/random_{g}.json")
    cl = load(f"{R}/crosslevel_{g}.json")
    clb = load(f"{B}/crosslevel_{g}.json")
    top = 6
    def cross(v):
        if v.startswith("DRGraph"):
            return clb["levels"][top]["drgraph"]["across_median_ratio"]
        if v.startswith("random"):
            return clb["levels"][top]["random"]["across_median_ratio"]
        return cl["levels"][top][v]["across_median_ratio"] if cl and v in cl["levels"][top] else None
    print("| layout | Spearman | top-region edges (L6, median ÷ random pair) | GT 10-NN | median ratio | mean ratio | p90 ÷ random median | NP₁ | NP₂ | GT k-means ARI |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for v, m in M.items():
        if not m:
            continue
        e, s, t = m["edges"], m.get("stress", {}), m.get("ground_truth", {})
        print(f"| {v} | {f(s.get('spearman_layout_vs_graph_distance'))} | {f(cross(v))} | {f(t.get('knn10_label_agreement'))} "
              f"| {f(e['median_ratio'])} | {f(e['mean_ratio'])} | {f(e['p90_over_random_median'])} | {f(m.get('np1'))} | {f(m.get('np2'))} | {f(t.get('kmeans_ari'))} |")
    print()
    lv = [q["level"] for q in M["hcl"]["quotient_np"] if q["np_top10"] is not None] if M["hcl"] else []
    print("NP@10 per quotient level\n")
    print("| layout | " + " | ".join(f"L{k}" for k in lv) + " |")
    print("|---|" + "---|" * len(lv))
    for v, m in M.items():
        if not m:
            continue
        q = {x["level"]: x["np_top10"] for x in m["quotient_np"]}
        print(f"| {v} | " + " | ".join(f(q.get(k)) for k in lv) + " |")
    print()
