#!/usr/bin/env python3
"""table.py RESULTS_DIR — Markdown tables per graph from results/{runs,metrics}/<layout>_<graph>.json."""
import glob, json, os, sys

R = sys.argv[1]
ORDER = ["hcl", "hcls", "hclg1", "hclg3", "drgraph", "fa2", "batchlayout", "sfdp", "fm3", "random"]


def load(d):
    out = {}
    for p in glob.glob(f"{R}/{d}/*.json"):
        nm, g = os.path.basename(p)[:-5].rsplit("_", 1)
        out.setdefault(g, {})[nm] = json.load(open(p))
    return out


def ordered(names):
    return [n for n in ORDER if n in names] + sorted(set(names) - set(ORDER))


def f(x, d=3):
    return "—" if x is None else f"{x:.{d}f}"


M, U = load("metrics"), load("runs")
for g in sorted(set(M) | set(U)):
    print(f"## {g}\n")
    m_g = M.get(g, {})
    if m_g:
        print("| layout | median ratio | mean ratio | p90 / random median | p90 / random p90 | edge CV | NP1 | NP2 | stress at a* | Spearman | GT silhouette | GT 10-NN agree | GT k-means ARI |")
        print("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
        for n in ordered(m_g):
            m = m_g[n]
            e, s, t = m["edges"], m.get("stress", {}), m.get("ground_truth", {})
            print(f"| {n} | {f(e['median_ratio'])} | {f(e['mean_ratio'])} | {f(e['p90_over_random_median'])} | {f(e['p90_ratio'])} | {f(e['edge_len_cv'], 2)} "
                  f"| {f(m.get('np1'))} | {f(m.get('np2'))} | {f(s.get('normalized_stress_at_optimal_scale'))} | {f(s.get('spearman_layout_vs_graph_distance'))} "
                  f"| {f(t.get('silhouette'))} | {f(t.get('knn10_label_agreement'))} | {f(t.get('kmeans_ari'))} |")
        print()
        lv = [q["level"] for q in next(iter(m_g.values())).get("quotient_np", [])]
        print("Quotient-level NP@10 (10 strongest quotient neighbours against the 10 nearest centroids); NP1 in brackets\n")
        print("| layout | " + " | ".join(f"L{k}" for k in lv) + " |")
        print("|---|" + "---|" * len(lv))
        for n in ordered(m_g):
            q = m_g[n].get("quotient_np", [])
            print(f"| {n} | " + " | ".join(f"{f(x['np_top10'])} ({f(x['np1'])})" for x in q) + " |")
        print()
    if U.get(g):
        print("| run | memory limit | rc | wall s | user s | peak RSS MB | peak anon MB (sampled) | threads | options |")
        print("|---|---|---|---|---|---|---|---|---|")
        for n in ordered(U[g]):
            u = U[g][n]
            print(f"| {n} | {u['memory_limit']} | {u['rc']} | {u['wall_s']} | {u['user_s']} | {u['peak_rss_mb']} | {u['peak_anon_mb_sampled']} | {u.get('threads', '')} | {u.get('options', '')} |")
        print()
