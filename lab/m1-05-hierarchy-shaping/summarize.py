#!/usr/bin/env python3
"""Table of the metrics files: summarize.py results/dev/*.json"""
import json
import sys

rows = []
for p in sys.argv[1:]:
    if ".stab" in p or ".build." in p or ".plain." in p or ".layout." in p:
        continue
    d = json.load(open(p))
    f, lf = d["fanout_all"], d["leaf_nodes"]
    bd = d["by_depth"]
    q = [(int(k), v.get("modularity_aggregates_resolved", v["modularity"])) for k, v in bd.items()]
    q1 = bd["1"].get("modularity_aggregates_resolved", bd["1"]["modularity"])
    rows.append([d["graph"], d["method"], d["tree_nodes"], d["depth_max"], f["median"], f["p95"], f["max"], round(f["share_parents_4_64"], 3), round(f["share_parents_2_3"], 3),
                 lf["median"], lf["max"], d["leaf_over_kmax"]["nodes"], round(d["share_nodes_under_aggregate"], 4), round(d["share_nodes_under_derived"], 4),
                 d.get("plain_modularity"), round(q1, 4), round(d["modularity_max_resolved"], 4), round(bd["1"]["conductance_node_weighted_communities"], 3),
                 round(bd.get("2", bd["1"])["conductance_node_weighted_communities"], 3), bd["1"]["regions"]])
hdr = ["graph", "method", "tree", "depth", "fan_p50", "fan_p95", "fan_max", "par_4_64", "par_2_3", "leaf_p50", "leaf_max", "nodes_leaf>kmax",
       "under_agg", "under_derived", "Q_plain", "Q_d1", "Q_best", "phi_d1", "phi_d2", "regions_d1"]
w = [max(len(str(r[i])) for r in rows + [hdr]) for i in range(len(hdr))]
for r in [hdr] + rows:
    print("  ".join(str(x).rjust(w[i]) for i, x in enumerate(r)))
