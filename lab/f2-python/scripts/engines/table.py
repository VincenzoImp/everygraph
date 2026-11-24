import json, glob, sys
R = "/home/tux/graphlab/exp/f2-python/results"
def m(r, *ks):
    for k in ks:
        v = r.get(k)
        if isinstance(v, dict) and "median_ms" in v:
            s = f"{v['median_ms']:.3g}"
            if v.get("timeouts"): s += f" ({v['timeouts']}/{v['n']} TO)"
            return s
        if isinstance(v, dict) and v.get("errors"): return "err"
    return "n/a"
def f(x): return "n/a" if x is None else f"{x:.3g}"
print("| engine | version | load s | disk MB | peak RSS MB | nbrs ms | 2-hop ms | <=3-hop ms | SP ms | PageRank s | WCC s | degree-dist s |")
print("|---|---|---|---|---|---|---|---|---|---|---|---|")
for e in ["igraph", "rustworkx", "ladybug", "duckpgq", "gfql", "neo4j"]:
    fs = glob.glob(f"{R}/goalA_{e}_lj.json")
    if not fs: continue
    r = json.load(open(fs[0]))
    rss = r.get("server_peak_rss_mb") or r.get("peak_rss_mb")
    print(f"| {r['engine']} | {r['version']} | {f(r.get('load_s'))} | {f(r.get('disk_mb'))} | {f(rss)} | {m(r,'neighbors')} | {m(r,'hop2')} | {m(r,'hop3')} | {m(r,'sp')} | {f(r.get('pagerank_s'))} | {f(r.get('wcc_s'))} | {f(r.get('degree_dist_s'))} |")
