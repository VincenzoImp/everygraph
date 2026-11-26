#!/usr/bin/env python3
"""table.py — Markdown tables from results/ of m1-07, with DRGraph and random from m1-18."""
import json, os

R = os.path.join(os.path.dirname(os.path.abspath(__file__)), "results")
B = os.path.join(os.path.dirname(R), "..", "m1-18-layout-baseline", "results")
OURS = ["hcl", "hcls", "hclsb", "cells", "cellsb", "cellsd"]


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

print("## Area (unit-square map; `results/layout/<variant>_<graph>.json`)\n")
print("| graph | layout | node discs | finest-region bubbles | bubbles at depth 1–7 | mean retention per depth | node radius p10 | zmax | cell area misallocated: mean / p90 / max | fallbacks | peak RSS MB (8 threads) |")
print("|---|---|---|---|---|---|---|---|---|---|---|")
for g in ["lj", "orkut"]:
    for v in OURS:
        j = load(f"{R}/layout/{v}_{g}.json")
        if not j:
            continue
        a = j["area"]
        bd = a["by_depth"]
        ret = [d["bubbles"] / d["parents_bubbles"] for d in bd[1:8] if d["parents_bubbles"]]
        mis = "—" if j["cells"] is None else f"{a['misallocated_mean']:.4f} / {a['misallocated_p90']:.4f} / {a['misallocated_max']:.2f}"
        fb = "—" if j["cells"] is None else a["cells_fallbacks"]
        print(f"| {g} | {v} | {a['node_discs']:.3f} | {a['finest_regions_bubbles']:.3f} | " + " ".join(f"{d['bubbles']:.2f}" for d in bd[1:8])
              + f" | {sum(ret) / len(ret):.2f} | {a['node_r_p10']:.1e} | {a['zmax_p10']:.0f} | {mis} | {fb} | {j['peak_rss_mb']:.0f} |")
print()

print("## Cost, exploratory (`results/determinism.txt`: layout time at 1 thread ÷ that of `hcls`, runs back to back on a shared machine)\n")
t = {}
for line in open(f"{R}/determinism.txt"):
    w = line.split()
    if len(w) == 4 and w[2] == "t1":
        t[(w[0], w[1])] = float(w[3])
print("| layout | lj | orkut |")
print("|---|---|---|")
for v in OURS:
    if ("lj", v) in t and ("orkut", v) in t:
        print(f"| {v} | {t['lj', v] / t['lj', 'hcls']:.2f} | {t['orkut', v] / t['orkut', 'hcls']:.2f} |")
print()

print("## Edges between top regions (`results/gap_<graph>.json`, `gap_extra_<graph>.json`)\n")
print("| graph | layout | L median | C median | G median | facing | share to 3 nearest regions | median rank of the other region |")
print("|---|---|---|---|---|---|---|---|")
for g in ["lj", "orkut"]:
    for fn in (f"{R}/gap_{g}.json", f"{R}/gap_extra_{g}.json"):
        j = load(fn)
        for v, x in (j or {}).get("layouts", {}).items():
            print(f"| {g} | {v} | {x['L_median']:.3f} | {x['C_median']:.3f} | {x['G_median']:.3f} | {x['facing_mean']:.3f} "
                  f"| {x['share_to_3_nearest_regions']:.3f} | {x['median_rank_of_other_region']:.0f} of {j['regions'] - 1} |")
print()
print("## How far groups of members could face their outside links (`results/facing_<graph>.json`, positions of `hcls`)\n")
print("| graph | group | raw bound | chance | bound without the region's common direction |")
print("|---|---|---|---|---|")
for g in ["lj", "orkut"]:
    j = load(f"{R}/facing_{g}.json")
    for x in (j or {}).get("groups", []):
        print(f"| {g} | {x['group']} | {x['facing_bound']:.3f} | {x['chance']:.3f} | {x['centred_bound']:.3f} |")
print()
print("## Tiles (`results/tiles/<variant>_<graph>.json`, `gmap tiles`, zmax chosen by the tiles stage)\n")
print("| graph | layout | zmax | tiles | file MB |")
print("|---|---|---|---|---|")
for g in ["lj", "orkut"]:
    for v in OURS:
        j = load(f"{R}/tiles/{v}_{g}.json")
        if j:
            print(f"| {g} | {v} | {j['zmax']} | {j['tiles']:,} | {j['file_bytes'] / 1e6:.0f} |")
print()
