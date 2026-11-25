#!/usr/bin/env python3
"""Table of the metrics files: summarize.py results/dev/*.json

Columns: fan-out of internal regions (median, max, share of parents with 4-64 and with 2-3
children), leaf communities (median and maximum node count, nodes in leaves over 384), share of
nodes under an aggregate, modularity of the plain partition, of the drawn top level (depth 1,
aggregates as single regions), of depth 1 with aggregates resolved, of the level T (role 2) and
of the two levels below T (roles 3, 4), node-weighted conductance of T+1 and T+2, and whether a
super level exists (its number of regions).
"""
import json
import sys

rows = []
for p in sys.argv[1:]:
    if not p.endswith(".json") or any(x in p for x in (".build.", ".plain.", ".layout.")):
        continue
    d = json.load(open(p))
    f, lf = d["fanout_all"], d["leaf_nodes"]
    bd, br = d["by_depth"], d.get("by_role", {})
    q = lambda k: round(br[k]["modularity"], 4) if k in br else "-"
    phi = lambda k: round(br[k]["conductance_node_weighted"], 3) if k in br else "-"
    rows.append([d["graph"], d["method"], d["depth_max"], f["median"], f["max"], round(f["share_parents_4_64"], 3), round(f["share_parents_2_3"], 3),
                 lf["median"], lf["max"], d["leaf_over_kmax"]["nodes"], round(d["share_nodes_under_aggregate"], 4),
                 d.get("plain_modularity"), round(bd["1"]["modularity"], 4), round(bd["1"].get("modularity_aggregates_resolved", bd["1"]["modularity"]), 4),
                 q("T"), q("T+1"), q("T+2"), phi("T+1"), phi("T+2"), br["super"]["regions"] if "super" in br else "-"])
hdr = ["graph", "variant", "depth", "fan_p50", "fan_max", "par_4_64", "par_2_3", "leaf_p50", "leaf_max", "nodes_leaf>384",
       "under_agg", "Q_plain", "Q_d1_drawn", "Q_d1_resolved", "Q_T", "Q_T+1", "Q_T+2", "phi_T+1", "phi_T+2", "super_regions"]
w = [max(len(str(r[i])) for r in rows + [hdr]) for i in range(len(hdr))]
for r in [hdr] + sorted(rows, key=lambda r: (r[0], r[1])):
    print("  ".join(str(x).rjust(w[i]) for i, x in enumerate(r)))
