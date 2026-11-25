#!/usr/bin/env python3
"""Tables for experiment #118 from results/*.jsonl. Writes results/summary.json, prints text tables.

A = raw arrays in memory, B = compressed store decoded on the fly, C = store decoded at open.
Every time is the best of the repetitions listed in the source row (`seconds_all` / `times_s`).
"""
import glob
import json
import os
import sys

R = sys.argv[1] if len(sys.argv) > 1 else "results"


def rows(name):
    p = os.path.join(R, name)
    if not os.path.exists(p):
        return []
    return [json.loads(l) for l in open(p) if l.strip().startswith("{")]


def fmt(x):
    if x is None:
        return "-"
    if x >= 100:
        return f"{x:.0f}"
    if x >= 10:
        return f"{x:.1f}"
    if x >= 1:
        return f"{x:.2f}"
    return f"{x:.3f}" if x >= 0.1 else f"{x:.4f}"


def ratio(a, b):
    return f"{a / b:.2f}x" if a and b else "-"


out = {}
GRAPHS = ["lj", "orkut", "twitter-sym", "friendster"]
CFG = {"raw": "A", "comp": "B", "decoded": "C"}

# ---------------------------------------------------------------- encode
print("== 1. Encoding into the default format (delta + Stream VByte records, Elias-Fano byte offsets)")
print("graph           bits/edge  (data + offsets + blocks)   raw   encode 16T s  M entries/s   encode 1T s   write+fsync s")
enc = {}
for r in rows("encode.jsonl"):
    enc.setdefault(r["graph"], {})[r["threads"]] = r
for g, e in enc.items():
    a = e.get(16) or next(iter(e.values()))
    one = e.get(1)
    print(f"{g:15s} {a['bits_per_edge']:6.2f}     ({a['bits_per_edge_data']:.2f} + {a['bits_per_edge_offsets_ef']:.2f} + {a['bits_per_edge_block_table']:.3f})"
          f"      {a['bits_per_edge_raw_u32_u64']:.1f}   {fmt(a['encode_seconds_best']):>8s}    {a['encode_edges_per_s'] / 1e6:8.0f}      "
          f"{fmt(one['encode_seconds_best']) if one else '-':>8s}       {fmt(a['write_seconds']):>6s}")
out["encode"] = {g: {str(t): {k: r[k] for k in ("bits_per_edge", "bits_per_edge_data", "bits_per_edge_offsets_ef", "bits_per_edge_raw_u32_u64",
                                               "encode_seconds_best", "write_seconds", "data_bytes", "offsets_ef_bytes", "loadavg_1m_start")}
                     for t, r in e.items()} for g, e in enc.items()}
ver = rows("verify.jsonl")
print(f"exactness: {sum(1 for v in ver if v.get('ok'))}/{len(ver)} stores decode to the input arrays on every access path, all three decoders")

# ---------------------------------------------------------------- kernels
kern = rows("kernels.jsonl") + rows("kernels_big.jsonl")
K = {}
for r in kern:
    K[(r["graph"], r["algo"], r["threads"], r["config"])] = r
ALGOS = ["pr32", "pr64", "wcc", "bfs", "bfs-ds", "kcore", "tc", "tc-comp"]
NAMES = {"pr32": "PageRank f32 x20", "pr64": "PageRank f64 x20", "wcc": "WCC (Afforest)", "bfs": "BFS (dir-opt)", "bfs-ds": "BFS, degree in state", "kcore": "k-core (PKC)",
         "tc": "triangles, fwd raw", "tc-comp": "triangles, fwd compressed"}
print("\n== 2. Kernels: seconds (best of reps), heap memory, warm; B/A and C/A are slowdown factors against A")
print("graph        kernel                     T      A raw     B comp   B/A      C decoded   C/A")
ktab = []
for g in GRAPHS:
    for algo in ALGOS:
        for t in (1, 16):
            a, b, c = (K.get((g, algo, t, cfg)) for cfg in ("raw", "comp", "decoded"))
            if not (a or b or c):
                continue
            sa, sb, sc = (x["seconds_best"] if x else None for x in (a, b, c))
            print(f"{g:12s} {NAMES[algo]:26s} {t:2d}  {fmt(sa):>9s}  {fmt(sb):>9s}  {ratio(sb, sa):>6s}  {fmt(sc):>9s}  {ratio(sc, sa):>6s}")
            ktab.append({"graph": g, "kernel": algo, "threads": t, "A_raw_s": sa, "B_comp_s": sb, "C_decoded_s": sc,
                         "B_over_A": sb / sa if sa and sb else None, "C_over_A": sc / sa if sa and sc else None,
                         "reps": {cfg: x["seconds_all"] for cfg, x in (("A", a), ("B", b), ("C", c)) if x},
                         "loadavg_1m_start": {cfg: x["loadavg_1m_start"] for cfg, x in (("A", a), ("B", b), ("C", c)) if x},
                         "busy_cpus_before": {cfg: x.get("busy_cpus_before") for cfg, x in (("A", a), ("B", b), ("C", c)) if x}})
out["kernels"] = ktab
# results must agree across configurations
bad = []
for (g, algo, t, cfg), r in K.items():
    ref = K.get((g, algo, t, "raw"))
    if not ref or cfg == "raw":
        continue
    res, rr = dict(r["result"]), dict(ref["result"])
    for d in (res, rr):
        # timings and sizes; and Afforest's "fraction examined", which depends on thread interleaving
        # at more than one thread (labels and component counts do not)
        for k in ("forward_build_seconds", "variant", "forward_bytes", "forward_bits_per_undirected_edge", "fraction_of_entries_examined"):
            d.pop(k, None)
    if "pr" in res:
        # PageRank sums floats in parallel in a schedule-dependent order: compare the top-5 node
        # ids exactly and the values to 1e-9 relative
        p, q = res["pr"], rr["pr"]
        same = ([i for i, _ in p["top5"]] == [i for i, _ in q["top5"]] and abs(p["sum"] - q["sum"]) < 1e-9
                and all(abs(x - y) <= 1e-9 * abs(y) for (_, x), (_, y) in zip(p["top5"], q["top5"])))
        if not same:
            bad.append((g, algo, t, cfg))
    elif res != rr:
        bad.append((g, algo, t, cfg))
print("result check (same output in every configuration; PageRank to 1e-9):", "all identical" if not bad else f"MISMATCH {bad}")
out["kernel_result_mismatches"] = bad

# ---------------------------------------------------------------- leiden
L = {}
for r in rows("leiden.jsonl") + rows("leiden_big.jsonl"):
    L[(r["graph"], "det" if r["det"] else "async", r["threads"], r["config"], r["mem"])] = r
print("\n== 3. Leiden, 1 iteration (seconds, best of repeats; modularity of the best run)")
print("graph        mode    T     A raw/heap   B comp/heap   B/A     C decoded   C/A     A raw/mmap   B comp/mmap    Q(A)    Q(B)   det hash equal")
ltab = []
for g in GRAPHS:
    for mode in ("async", "det"):
        for t in (1, 16):
            a, b, c, am, bm = (L.get((g, mode, t, cfg, mem)) for cfg, mem in (("raw", "heap"), ("comp", "heap"), ("decoded", "heap"), ("raw", "mmap"), ("comp", "mmap")))
            if not (a or b):
                continue
            s = [x["time_s"] if x else None for x in (a, b, c, am, bm)]
            heq = "-"
            if mode == "det":
                hs = {x["hash_flat"] for x in (a, b, c, am, bm) if x}
                heq = "yes" if len(hs) == 1 else "NO"
            print(f"{g:12s} {mode:6s} {t:2d}  {fmt(s[0]):>9s}    {fmt(s[1]):>9s}    {ratio(s[1], s[0]):>6s}  {fmt(s[2]):>9s}  {ratio(s[2], s[0]):>6s}  {fmt(s[3]):>9s}    {fmt(s[4]):>9s}"
                  f"    {a['modularity'] if a else float('nan'):.4f}  {b['modularity'] if b else float('nan'):.4f}   {heq}")
            ltab.append({"graph": g, "mode": mode, "threads": t, "A_raw_heap_s": s[0], "B_comp_heap_s": s[1], "C_decoded_heap_s": s[2], "A_raw_mmap_s": s[3], "B_comp_mmap_s": s[4],
                         "B_over_A": s[1] / s[0] if s[0] and s[1] else None, "modularity_A": a and a["modularity"], "modularity_B": b and b["modularity"],
                         "det_hash_equal": heq, "times": {k: x["times_s"] for k, x in (("A", a), ("B", b), ("C", c)) if x},
                         "phases_A": a and {k: a[k] for k in ("lm_s", "refine_s", "agg_s")}, "phases_B": b and {k: b[k] for k in ("lm_s", "refine_s", "agg_s")},
                         "anon_peak_gb": {k: x["rss_anon_peak_kb"] / 1048576 for k, x in (("A", a), ("B", b)) if x}})
out["leiden"] = ltab

# ---------------------------------------------------------------- point queries
P = {}
for r in rows("pq.jsonl"):
    P[(r["graph"], r["config"], r["mem"])] = r
QN = ["degree", "neighbors", "common_neighbors", "two_hop_size", "khop3_budget10k", "bidir_bfs_path"]
print("\n== 4. Point queries: warm, 1 thread, microseconds p50 / p95 / p99 (per-query timer inside); [batch mean without timers]")
ptab = []
for g in GRAPHS + ["twitter-dir"]:
    for q in QN:
        line = []
        rec = {"graph": g, "query": q}
        for cfg, mem in (("raw", "mmap"), ("raw", "heap"), ("comp", "mmap"), ("comp", "heap"), ("decoded", "heap")):
            r = P.get((g, cfg, mem))
            if not r or q not in r["queries"]:
                continue
            v = r["queries"][q]
            bm = f" [{v['batch_mean_us']:.3f}]" if "batch_mean_us" in v else ""
            line.append(f"{CFG[cfg]} {cfg}/{mem}: {fmt(v['p50_us'])} / {fmt(v['p95_us'])} / {fmt(v['p99_us'])}{bm}")
            rec[f"{cfg}/{mem}"] = {k: v[k] for k in ("n", "p50_us", "p95_us", "p99_us", "mean_us", "batch_mean_us", "result_digest") if k in v}
        if line:
            print(f"{g:12s} {q:17s} " + "   ".join(line))
            ptab.append(rec)
out["point_queries"] = ptab
digests = {}
for rec in ptab:
    ds = {v["result_digest"] for k, v in rec.items() if isinstance(v, dict)}
    if len(ds) > 1:
        digests[(rec["graph"], rec["query"])] = list(ds)
print("result check (same answers in every configuration):", "all identical" if not digests else f"MISMATCH {digests}")

# ---------------------------------------------------------------- open
print("\n== 5. Opening: seconds at 16 threads unless noted (best of reps); decode = configuration C")
O = {}
for r in rows("open.jsonl"):
    O.setdefault((r["graph"], r["what"], r["mem"], r["threads"], r["cold"]), []).append(r)
otab = []
for g in GRAPHS:
    for key in sorted(k for k in O if k[0] == g):
        rs = O[key]
        best = min(rs, key=lambda r: r["total_seconds"])
        extra = ""
        if key[1] == "decoded":
            extra = (f"  open {fmt(best['open_seconds'])} + decode {fmt(best['decode_seconds'])} ({best['decode_edges_per_s'] / 1e9:.2f} G entries/s)"
                     f"  raw copy {best['raw_bytes_in_ram'] / 2**30:.2f} GiB anon, peak RSS {best['peak_rss_gb']:.2f} GiB")
        print(f"{g:12s} {key[1]:8s} mem={key[2]:9s} T={key[3]:2d} {'cold' if key[4] else 'warm'}  total {fmt(best['total_seconds']):>7s}{extra}")
        otab.append({"graph": g, "what": key[1], "mem": key[2], "threads": key[3], "cold": key[4], "total_seconds_best": best["total_seconds"],
                     "total_seconds_all": [r["total_seconds"] for r in rs], "open_seconds": best["open_seconds"], "decode_seconds": best.get("decode_seconds"),
                     "peak_rss_gb": best["peak_rss_gb"], "raw_bytes_in_ram": best.get("raw_bytes_in_ram"), "file_bytes": best["file_bytes"],
                     "resident_fraction_after_evict": best.get("resident_fraction_after_evict")})
out["open"] = otab

# ---------------------------------------------------------------- scan
print("\n== 6. Scans (sum of all targets) and random access to one list")
print("graph        config      decoder  batch  T    G entries/s   random list ns   random degree ns")
stab = []
for r in rows("scan.jsonl"):
    print(f"{r['graph']:12s} {r['config']:11s} {r['decoder'] if r['config'] != 'raw' else '-':7s} {str(r.get('scan_batch_entries', '-')):>6s} {r['threads']:2d}   {r['edges_per_s'] / 1e9:8.2f}      {r['random_list_ns']:9.0f}      {r['random_degree_ns']:9.1f}")
    stab.append({k: r.get(k) for k in ("graph", "config", "decoder", "scan_batch_entries", "threads", "edges_per_s", "random_list_ns", "random_degree_ns", "seconds_all", "loadavg_1m_start", "busy_cpus_before")})
out["scan"] = stab

# ---------------------------------------------------------------- ablations
print("\n== 7. Ablations (seconds, best of reps)")
atab = []
for r in rows("ablation.jsonl"):
    if r.get("cmd") == "kernel":
        base = K.get((r["graph"], r["algo"], r["threads"], "raw"))
        comp = K.get((r["graph"], r["algo"], r["threads"], "comp"))
        b = base["seconds_best"] if base else None
        c = comp["seconds_best"] if comp else None
        tag = f"{r.get('label', '')} {r['config']} mem={r['mem']} decoder={r['decoder']} batch={r.get('scan_batch_entries', '-')} degidx={r.get('degree_index', '-')}"
        extra = ""
        if r["algo"] == "tc-naive":
            extra = f"  triangles {r['result']['triangles']}  sum deg^2 {r['result']['sum_deg_squared']:.3e}"
        if r["algo"] == "degsq":
            extra = f"  sum deg^2 {r['result']['sum_deg_squared']:.3e}"
        print(f"{r['graph']:12s} {r['algo']:8s} T={r['threads']:2d} {tag:70s} {fmt(r['seconds_best']):>8s}  vs A {ratio(r['seconds_best'], b):>7s}  vs B {ratio(r['seconds_best'], c):>7s}{extra}")
        atab.append({"graph": r["graph"], "algo": r["algo"], "threads": r["threads"], "label": r.get("label"), "config": r["config"], "mem": r["mem"], "decoder": r["decoder"],
                     "batch": r.get("scan_batch_entries"), "degree_index": r.get("degree_index"), "seconds_best": r["seconds_best"], "seconds_all": r["seconds_all"], "A_raw_s": b, "B_comp_s": c,
                     "result": r["result"] if r["algo"] in ("tc-naive", "degsq") else None})
    elif r.get("cmd") == "pq":
        for q, v in r["queries"].items():
            print(f"{r['graph']:12s} pq {q:17s} {r.get('label', '')} {r['config']}: p50 {fmt(v['p50_us'])} us  p99 {fmt(v['p99_us'])} us")
            atab.append({"graph": r["graph"], "query": q, "label": r.get("label"), "config": r["config"], "p50_us": v["p50_us"], "p99_us": v["p99_us"], "n": v["n"]})
out["ablation"] = atab

# ---------------------------------------------------------------- capped
cap = rows("capped.jsonl")
if cap:
    print("\n== 8. Under a hard memory limit (16 threads, files evicted first, mmap without pre-fault)")
    ctab = []
    for r in cap:
        if r.get("cmd") == "kernel":
            print(f"{r['graph']:12s} {r.get('label', ''):15s} {r['config']:8s} {r['algo']:5s} {fmt(r['seconds_best']):>8s} s  (reps {[round(x, 2) for x in r['seconds_all']]}, iters {r['result'].get('iters', '-')})"
                  f"  anon {r['rss_anon_gb']:.2f} GiB peak RSS {r['peak_rss_gb']:.2f} GiB")
            ctab.append({k: r.get(k) for k in ("graph", "label", "config", "mem", "algo", "threads", "seconds_best", "seconds_all", "rss_anon_gb", "peak_rss_gb")} | {"iters": r["result"].get("iters")})
        else:
            print(f"{r['graph']:12s} MemoryMax={r['limit']:4s} {r['config']:8s} {r['algos']:14s} exit code {r['exit_code']}")
            ctab.append(r)
    out["capped"] = ctab

# ---------------------------------------------------------------- perf
pf = rows("perf.jsonl")
if pf:
    print("\n== 9. perf stat, user space, whole process including graph load (billions of events; counters were multiplexed and scaled by perf)")
    print("   cache-miss = perf's generic cache-misses event; perf reports L1d-miss and dTLB-miss without the :u suffix")
    print(f"{'label':32s} {'cycles':>9s} {'instr':>9s} {'IPC':>5s} {'br-miss':>9s} {'L1d-miss':>9s} {'cache-miss':>10s} {'dTLB-miss':>9s}  kernel s (best x reps)")
    ptab2 = []
    for r in sorted(pf, key=lambda r: r["label"]):
        e = r["counters_user_space_whole_process"]
        g = lambda k: (e.get(k + ":u") or e.get(k) or 0.0) / 1e9
        cyc, ins = g("cycles"), g("instructions")
        ks = r.get("kernel_seconds_all") or []
        print(f"{r['label']:32s} {cyc:9.2f} {ins:9.2f} {ins / cyc if cyc else 0:5.2f} {g('branch-misses'):9.3f} {g('L1-dcache-load-misses'):9.3f} {g('cache-misses'):10.3f} {g('dTLB-load-misses'):9.4f}  {fmt(min(ks)) if ks else '-'} x{len(ks)}")
        ptab2.append(r)
    out["perf"] = ptab2

# ---------------------------------------------------------------- baseline (lab prototypes as they are)
b1 = rows("baseline_b1.jsonl")
b2 = rows("baseline_b2.jsonl")
if b1 or b2:
    print("\n== 10. The lab prototypes re-run unchanged (for comparison with docs/FINDINGS.md 'Idle-machine re-runs')")
    btab = []
    for r in b1:
        per = f" ({r['seconds_best'] / r['result']['iters']:.2f} s/iter)" if r["algo"].startswith("pr") and r["graph"] == "friendster" else ""
        mine = K.get((r["graph"], r["algo"], r["threads"], "raw"))
        print(f"b1-core  {r['graph']:11s} {r['algo']:6s} T={r['threads']:2d} load={r['load']:5s} {fmt(r['seconds_best']):>8s} s{per}   this harness, A: {fmt(mine['seconds_best']) if mine else '-'}")
        btab.append({"tool": "b1-core", "graph": r["graph"], "algo": r["algo"], "threads": r["threads"], "load": r["load"], "seconds_best": r["seconds_best"],
                     "seconds_all": r["seconds_all"], "harness_A_s": mine and mine["seconds_best"], "loadavg_1m_start": r.get("loadavg_1m_start")})
    for r in b2:
        mode = "det" if r["det"] else "async"
        mine = L.get((r["graph"], mode, r["threads"], "raw", "mmap"))
        mineh = L.get((r["graph"], mode, r["threads"], "raw", "heap"))
        print(f"b2-leiden {r['graph']:10s} {mode:5s} T={r['threads']:2d} {fmt(r['time_s']):>8s} s  Q {r['modularity']:.4f}   this harness, A mmap: {fmt(mine['time_s']) if mine else '-'}  A heap: {fmt(mineh['time_s']) if mineh else '-'}")
        btab.append({"tool": "b2-leiden", "graph": r["graph"], "mode": mode, "threads": r["threads"], "time_s": r["time_s"], "times_s": r["times_s"], "modularity": r["modularity"],
                     "harness_A_mmap_s": mine and mine["time_s"], "harness_A_heap_s": mineh and mineh["time_s"], "loadavg_1m_start": r.get("loadavg_1m_start")})
    for f in sorted(glob.glob(os.path.join(R, "baseline_f1_*.json"))):
        r = json.load(open(f))
        for q, v in r["warm"].items():
            mine = P.get((r["graph"], "raw", "mmap"))
            mv = mine["queries"].get(q) if mine else None
            print(f"f1-query {r['graph']:11s} {q:17s} p50 {fmt(v['p50_us'])} / p95 {fmt(v['p95_us'])} / p99 {fmt(v['p99_us'])} us (n={v['n']})   this harness, A mmap p50: {fmt(mv['p50_us']) if mv else '-'}")
            btab.append({"tool": "f1-query", "graph": r["graph"], "query": q, "n": v["n"], "p50_us": v["p50_us"], "p95_us": v["p95_us"], "p99_us": v["p99_us"], "harness_A_mmap_p50_us": mv and mv["p50_us"]})
    out["baseline"] = btab

json.dump(out, open(os.path.join(R, "summary.json"), "w"), indent=1)
