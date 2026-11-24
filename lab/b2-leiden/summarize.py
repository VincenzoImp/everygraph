#!/usr/bin/env python3
"""Collect results/**.json into results/summary.json and print markdown tables (stdout)."""
import json, glob, os, collections, re
os.chdir(os.path.dirname(os.path.abspath(__file__)))
out = {}

def load(pattern):
    r = []
    for f in sorted(glob.glob(pattern)):
        try:
            j = json.load(open(f)); j["_file"] = os.path.basename(f)[:-5]; r.append(j)
        except Exception as e:
            print(f"<!-- skip {f}: {e} -->")
    return r

def scaling(title, pattern, threads, key):
    runs = load(pattern)
    if not runs: return
    rows = collections.defaultdict(dict)
    for j in runs:
        rows[(j["graph"], j["algo"], "det" if j["det"] else "async")][j["threads"]] = j
    print(f"\n## {title}\n")
    print("| graph | algo | mode | " + " | ".join(f"T={t}" for t in threads) + " | best M entries/s | modularity | comms | disconnected | passes | LM iters | lvl-0 edge passes |")
    print("|---|---|---|" + "---|" * (len(threads) + 7))
    o = {"runs": [], "determinism": [], "det_over_async": []}
    for (g, a, m), r in sorted(rows.items()):
        ts = [f"{r[t]['time_s']:.2f}" if t in r else "-" for t in threads]
        best = max(r.values(), key=lambda j: j["edges_per_s"])
        ref = r[max(r)]
        print(f"| {g} | {a} | {m} | " + " | ".join(ts) + f" | {best['edges_per_s']/1e6:.0f} (T={best['threads']}) | {ref['modularity']:.4f} | {ref['communities']} | {ref['disconnected']} | {ref['n_passes']} | {ref['total_iters']} | {ref['level0_edge_passes']:.1f} |")
        for t, j in sorted(r.items()):
            o["runs"].append({k: j.get(k) for k in ("graph", "algo", "det", "threads", "time_s", "times_s", "edges_per_s", "lm_s", "refine_s", "agg_s", "modularity", "communities", "disconnected", "n_passes", "total_iters", "level_sizes", "hash_levels", "rss_peak_kb", "rss_anon_peak_kb", "level0_edge_passes", "state_bytes_per_node")})
        if m == "det":
            hs = {t: j["hash_levels"] for t, j in sorted(r.items())}
            o["determinism"].append({"graph": g, "algo": a, "hashes": hs, "identical": len(set(hs.values())) == 1})
    print("\nDeterministic mode — hash of all level maps per thread count:\n")
    for d in o["determinism"]:
        print(f"- {d['graph']} {d['algo']}: identical={d['identical']} " + " ".join(f"T{t}={h}" for t, h in d["hashes"].items()))
    print("\nPrice of determinism (det time / async time, same thread count):\n")
    print("| graph | algo | " + " | ".join(f"T={t}" for t in threads) + " | modularity det vs async |")
    print("|---|---|" + "---|" * (len(threads) + 1))
    for (g, a, m), r in sorted(rows.items()):
        if m != "det": continue
        ra = rows.get((g, a, "async"), {})
        ratios = {t: r[t]["time_s"] / ra[t]["time_s"] for t in threads if t in r and t in ra}
        o["det_over_async"].append({"graph": g, "algo": a, "ratios": ratios})
        qa = ra[max(ra)]["modularity"] if ra else float("nan")
        print(f"| {g} | {a} | " + " | ".join(f"{ratios[t]:.2f}x" if t in ratios else "-" for t in threads) + f" | {r[max(r)]['modularity']:.4f} vs {qa:.4f} |")
    out[key] = o

def simple(title, pattern, key, hier=False):
    runs = load(pattern)
    if not runs: return
    print(f"\n## {title}\n")
    if not hier:
        print("| graph | run | threads | time s | modularity | communities | disconnected | passes | hash |")
        print("|---|---|---|---|---|---|---|---|---|")
    else:
        print("| graph | variant | T | time s | top modularity | disconnected | levels | per level: mean / p50 / p90 / max fan-out, share of children under parents with 4..64 children |")
        print("|---|---|---|---|---|---|---|---|")
    o = []
    for j in runs:
        name = j["_file"].split("_", 1)[1]
        o.append({"run": name, **{k: j.get(k) for k in ("graph", "threads", "time_s", "modularity", "communities", "disconnected", "hash_levels", "level_sizes", "fanout", "n_passes")}})
        if not hier:
            print(f"| {j['graph']} | {name} | {j['threads']} | {j['time_s']:.2f} | {j['modularity']:.4f} | {j['communities']} | {j['disconnected']} | {j['n_passes']} | {j['hash_levels']} |")
        else:
            fo = "; ".join(f"{x['mean']:.1f}/{x['p50']}/{x['p90']}/{x['max']}, {x['children_in_parents_4_64']:.2f}" for x in j["fanout"])
            print(f"| {j['graph']} | {name} | {j['threads']} | {j['time_s']:.2f} | {j['modularity']:.4f} | {j['disconnected']} | {j['n']} → " + " → ".join(map(str, j["level_sizes"])) + f" | {fo} |")
    out[key] = o

scaling("Thread scaling, whole machine under bench.lock (best-of-N wall seconds, graph load excluded)", "results/scaling/*.json", (1, 2, 4, 8, 16, 32), "scaling")
scaling("Thread scaling on the 3 assigned dev cores only (taskset 6-8 [+22-24 for T=6], no lock, machine shared)", "results/dev_scaling/*.json", (1, 2, 3, 6), "dev_scaling")
simple("Quality mode / order ablation (whole machine)", "results/quality/*.json", "quality")
simple("Quality mode / order ablation (dev cores, timings not headline)", "results/dev_quality/*.json", "dev_quality")
simple("Hierarchies (whole machine)", "results/hier/*.json", "hier", hier=True)
simple("Hierarchies (dev cores)", "results/dev_hier/*.json", "dev_hier", hier=True)

dh = load("results/dev_hash/*.json")
if dh:
    print("\n## Determinism at high thread counts (4/16/32 worker threads confined to 6 dev hw threads; hashes only)\n")
    grp = collections.defaultdict(dict)
    for j in dh: grp[(j["graph"], j["algo"])][j["threads"]] = j["hash_levels"]
    out["dev_hash"] = []
    for (g, a), hs in sorted(grp.items()):
        out["dev_hash"].append({"graph": g, "algo": a, "hashes": hs, "identical": len(set(hs.values())) == 1})
        print(f"- {g} {a} det: identical={len(set(hs.values())) == 1} " + " ".join(f"T{t}={h}" for t, h in sorted(hs.items())))

refs = load("results/refs/*.json")
if refs:
    print("\n## Reference partitions (same eval routine: `b2 eval`)\n")
    print("| graph | partition | modularity | communities | disconnected | largest | ref wall s |")
    print("|---|---|---|---|---|---|---|")
    out["refs"] = []
    for j in refs:
        name = j["_file"].split("_", 1)[1]
        p = f"ref/out/{j['graph']}.{name}.json"
        tj = json.load(open(p)) if os.path.exists(p) else {}
        out["refs"].append({"partition": name, "ref_seconds": tj.get("seconds"), **{k: j[k] for k in ("graph", "modularity", "communities", "disconnected", "largest")}})
        print(f"| {j['graph']} | {name} | {j['modularity']:.4f} | {j['communities']} | {j['disconnected']} | {j['largest']} | {tj.get('seconds', float('nan')):.1f} |")

for bigpat, bigtitle, bigkey in (("results/big/*.json", "Large graphs (32 threads, --lowmem two-pass aggregation, mmap without populate, f32 weights)", "big"), ("results/semiext/*.json", "Semi-external analogue on orkut (6 dev threads, private page-cache-evicted copy of the CSR, hard cgroup limit; targets.u32 = 0.94 GB)", "semiext")):
    big = load(bigpat)
    if not big: continue
    print(f"\n## {bigtitle}\n")
    print("| run | n | entries | time s | LM s | refine s | agg s | M entries/s | modularity | comms | disconnected | levels | lvl-0 edge passes | VmHWM GB | anon peak GB | cgroup peak GB (limit) | major faults |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    out[bigkey] = []
    for j in big:
            cg = j.get("cgroup", {})
            out[bigkey].append({k: j.get(k) for k in ("_file", "graph", "algo", "n", "entries", "time_s", "lm_s", "refine_s", "agg_s", "edges_per_s", "modularity", "communities", "disconnected", "level_sizes", "level0_edge_passes", "rss_peak_kb", "rss_anon_peak_kb", "cgroup", "passes")})
            print(f"| {j['_file']} | {j['n']} | {j['entries']} | {j['time_s']:.1f} | {j['lm_s']:.1f} | {j['refine_s']:.1f} | {j['agg_s']:.1f} | {j['edges_per_s']/1e6:.0f} | {j['modularity']:.4f} | {j['communities']} | {j['disconnected']} | " + " → ".join(map(str, j["level_sizes"])) + f" | {j['level0_edge_passes']:.1f} | {j['rss_peak_kb']/1048576:.1f} | {j['rss_anon_peak_kb']/1048576:.1f} | {cg.get('memory_peak', 0)/2**30:.1f} ({cg.get('memory_max')}) | {cg.get('pgmajfault')} |")

gve = sorted(glob.glob("results/gve/*.log"))
if gve:
    print("\n## GVE reference binaries (built by exp/a-baselines, run here under bench.lock, 32 threads)\n")
    out["gve"] = []
    for f in gve:
        txt = open(f, errors="replace").read()
        lines = [l for l in txt.splitlines() if re.search(r"ms|modularity", l)]
        out["gve"].append({"file": os.path.basename(f), "lines": lines[-12:]})
        print(f"**{os.path.basename(f)}**\n```\n" + "\n".join(lines[-8:]) + "\n```")
json.dump(out, open("results/summary.json", "w"), indent=1)
