"""Summarize results/*.jsonl into a size-vs-speed table and chart-ready JSON (results/summary.json)."""
import json, sys, os

R = sys.argv[1] if len(sys.argv) > 1 else "results"
PFX = sys.argv[2] if len(sys.argv) > 2 else ""


def load(name):
    p = os.path.join(R, PFX + name)
    if not os.path.exists(p):
        return []
    out = []
    for l in open(p):
        l = l.strip()
        if l.startswith("{"):
            out.append(json.loads(l))
    # keep the last run per (prefix, codec, threads, mode) — re-runs supersede earlier ones
    seen = {}
    for d in out:
        seen[(d["prefix"], d["codec"], d["threads"], "cold_scan_s" in d)] = d
    return list(seen.values())


def gname(d):
    p = d["prefix"]
    for k, v in [("enc/orkut", "orkut"), ("enc/uk", "uk-2005"), ("enc/twsym", "twitter-sym"), ("g/twitter", "twitter-dir")]:
        if k in p:
            return v
    return p


def cname(d):
    b = os.path.basename(d["prefix"])
    if d["codec"] == "bvs":
        return {"bv7": "BV w7 r3 (default)", "bv0": "BV w0 (no refs)", "tw": "BV (LAW file)"}.get(b, b)
    if d["codec"] == "bv":
        return "BV w7 r3 dyn-dispatch"
    return {"raw": "raw u32 CSR", "varint": "delta+LEB128", "svb": "delta+StreamVByte", "bp": "delta+BP128/BP32",
            "zstd3": "zstd-3 64KiB chunks", "lz4": "lz4 64KiB chunks", "vzstd3": "delta+LEB128+zstd3 (64K-edge chunks)"}.get(d["codec"], d["codec"])


M = 1e6
summary = {"bakeoff": [], "cold": [], "cold8g": [], "order": []}
print("== warm bake-off (32 threads unless noted; machine shared/noisy)")
print(f"{'graph':12} {'codec':24} {'bits/e':>7} {'scan1T Me/s':>11} {'scan32T Me/s':>12} {'PR it s':>8} {'PR Me/s':>8} {'rand ns/list':>12} {'ns/edge':>8}")
for d in load("bakeoff.jsonl"):
    row = dict(graph=gname(d), codec=cname(d), bits_per_edge=d["bits_per_edge"], bits_per_edge_data=d["bits_per_edge_data_only"],
               scan_1t_edges_per_s=d["scan_1t_edges_per_s"], scan_par_edges_per_s=d["scan_par_edges_per_s"],
               pr_iter_s=d["pr_iter_s"], pr_edges_per_s=d["pr_edges_per_s"], rand_ns_per_list=d["rand_ns_per_list"],
               rand_ns_per_edge=d["rand_ns_per_edge"], rand_par_ns_per_list=d["rand_par_ns_per_list"], threads=d["threads"],
               bytes=d["data_bytes"] + d["index_bytes"])
    summary["bakeoff"].append(row)
    print(f"{row['graph']:12} {row['codec']:24} {row['bits_per_edge']:7.2f} {row['scan_1t_edges_per_s']/M:11.0f} {row['scan_par_edges_per_s']/M:12.0f} {row['pr_iter_s']:8.3f} {row['pr_edges_per_s']/M:8.0f} {row['rand_ns_per_list']:12.0f} {row['rand_ns_per_edge']:8.1f}")
for f, key in [("cold.jsonl", "cold"), ("cold8g.jsonl", "cold8g")]:
    rows = load(f)
    if not rows:
        continue
    print(f"\n== cold scan ({f}; 32 threads; after posix_fadvise DONTNEED)")
    print(f"{'graph':12} {'codec':24} {'bits/e':>7} {'GB':>6} {'cold Me/s':>10} {'cold s':>7} {'rescan Me/s':>11} {'resident pages before':>21}")
    for d in rows:
        b = d["data_bytes"] + d["index_bytes"]
        row = dict(graph=gname(d), codec=cname(d), bits_per_edge=d.get("bits_per_edge"), bytes=b,
                   cold_edges_per_s=d["cold_scan_edges_per_s"], cold_s=d["cold_scan_s"],
                   rescan_edges_per_s=d["after_cold_rescan_edges_per_s"], resident_before=d["cold_resident_before"])
        summary[key].append(row)
        print(f"{row['graph']:12} {row['codec']:24} {row['bits_per_edge']:7.2f} {b/1e9:6.2f} {row['cold_edges_per_s']/M:10.0f} {row['cold_s']:7.2f} {row['rescan_edges_per_s']/M:11.0f} {row['resident_before']:21}")
rows = load("order.jsonl")
if rows:
    print("\n== node ordering (orkut)")
    print(f"{'order':8} {'codec':24} {'bits/e':>7} {'scan1T':>7} {'scanP':>7} {'PR s':>7} {'rand ns':>8}")
    for d in rows:
        p = d["prefix"]
        b = os.path.basename(p)
        order = "orig" if "enc/orkut" in p else b.split(".")[0]
        row = dict(order=order, codec=cname(dict(d, prefix=p.replace(order + ".", ""))), bits_per_edge=d["bits_per_edge"],
                   scan_1t_edges_per_s=d["scan_1t_edges_per_s"], scan_par_edges_per_s=d["scan_par_edges_per_s"],
                   pr_iter_s=d["pr_iter_s"], rand_ns_per_list=d["rand_ns_per_list"], threads=d["threads"])
        summary["order"].append(row)
        print(f"{order:8} {row['codec']:24} {row['bits_per_edge']:7.2f} {row['scan_1t_edges_per_s']/M:7.0f} {row['scan_par_edges_per_s']/M:7.0f} {row['pr_iter_s']:7.3f} {row['rand_ns_per_list']:8.0f}")
json.dump(summary, open(os.path.join(R, PFX + "summary.json"), "w"), indent=1)
