#!/usr/bin/env python3
"""Summarize ab.py rows: determinism check, per-variant medians, in-block ratios.

    python3 summarize.py results/ab-lj-orkut.jsonl [--json results/summary-lj-orkut.json]

Times are medians over rounds of b2's in-process kernel time (`time_s`: excludes loading the
graph and the final modularity evaluation). Ratios are taken inside a block (same round,
graph, threads, iterations) and then the median over rounds is reported.
"""
import argparse
import json
import statistics as st
from collections import defaultdict

ORDER = ["auto", "s4", "s8", "s16", "s32", "s64", "L16k", "L64k", "L64k-max16", "async"]


def med(xs):
    return st.median(xs) if xs else float("nan")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--json")
    ap.add_argument("--counts", help="rows of counts.py: parallel / one-thread sub-rounds per run")
    ap.add_argument("--seeds", help="rows of seeds.sh: modularity per --bucket-shift")
    a = ap.parse_args()
    cnt = {}
    if a.counts:
        with open(a.counts) as f:
            for l in f:
                c = json.loads(l)
                if not c.get("tag"):
                    cnt[(c["graph"], c["iterations"], c["variant"])] = c
    rows = []
    for fn in a.files:
        with open(fn) as f:
            rows += [json.loads(l) for l in f]
    bad = [r for r in rows if r["status"] != "ok"]
    rows = [r for r in rows if r["status"] == "ok"]
    by = defaultdict(list)  # (g, it, v, t) -> rows
    blk = {}  # (round, g, t, it, v) -> row
    for r in rows:
        by[(r["graph"], r["iterations"], r["variant"], r["threads"])].append(r)
        blk[(r["round"], r["graph"], r["threads"], r["iterations"], r["variant"])] = r
    graphs = sorted({k[0] for k in by}, key=lambda g: ["lj", "orkut", "friendster"].index(g) if g in ["lj", "orkut", "friendster"] else 9)
    its = sorted({k[1] for k in by})
    ths = sorted({k[3] for k in by})
    vs = [v for v in ORDER if any(k[2] == v for k in by)]
    out = {"determinism": [], "table": [], "failed": [{k: r.get(k) for k in ("round", "graph", "threads", "iterations", "variant", "status")} for r in bad]}

    print("== Determinism: hash_levels per (graph, iterations, variant) over all thread counts and rounds")
    for g in graphs:
        for it in its:
            for v in vs:
                if v == "async":
                    continue
                hs = defaultdict(set)
                for t in ths:
                    for r in by.get((g, it, v, t), []):
                        hs[r["hash_levels"]].add(t)
                if not hs:
                    continue
                ok = len(hs) == 1
                out["determinism"].append({"graph": g, "iterations": it, "variant": v, "identical": ok, "hashes": {h: sorted(t) for h, t in hs.items()}})
                print(f"  {g:6} it{it} {v:>10}: {'IDENTICAL' if ok else 'DIFFERENT'} {dict((h, sorted(t)) for h, t in hs.items())}")

    print("\n== Per variant (medians over rounds). ratio_async: det time / async time at the same threads, in-block;"
          " ratio_auto: time / auto time, in-block")
    hdr = f"{'graph':6} {'it':>2} {'variant':>10} {'thr':>3} {'n':>2} {'time_s':>7} {'lm_s':>6} {'ref_s':>6} {'agg_s':>6} {'r_async':>7} {'r_auto':>6} {'speedup':>7} {'Q':>8} {'comms':>6} {'disc':>4} {'passes':>6} {'iters':>5} {'capped':>6} {'work':>5} {'par_sr':>6} {'seq_sr':>6} {'subs':>s}"
    print(hdr)
    for g in graphs:
        for it in its:
            for v in vs:
                for t in ths:
                    rs = by.get((g, it, v, t), [])
                    if not rs:
                        continue
                    ra, rau = [], []
                    for r in rs:
                        a_ = blk.get((r["round"], g, t, it, "async"))
                        u_ = blk.get((r["round"], g, t, it, "auto"))
                        if a_:
                            ra.append(r["time_s"] / a_["time_s"])
                        if u_:
                            rau.append(r["time_s"] / u_["time_s"])
                    t1 = [r1["time_s"] for r1 in by.get((g, it, v, 1), [])]
                    sp = med(t1) / med([r["time_s"] for r in rs]) if t1 else float("nan")
                    r0 = rs[0]
                    work = sum(p["edge_passes"] * p["entries"] for p in r0["passes"]) / r0["entries"]
                    c = cnt.get((g, it, v))
                    if c:
                        assert c["hash_levels"] == r0["hash_levels"], (g, it, v)
                    cpar, cseq = (c["subrounds_par"], c["subrounds_seq"]) if c else (None, None)
                    capped = sum(1 for p in r0["passes"] if p["iters"] >= 20)
                    subs = sorted({p.get("sub", 0) for p in r0["passes"]}, reverse=True)
                    qs = {r["modularity"] for r in rs}
                    e = {
                        "graph": g, "iterations": it, "variant": v, "threads": t, "n_runs": len(rs),
                        "time_s": med([r["time_s"] for r in rs]), "lm_s": med([r["lm_s"] for r in rs]),
                        "refine_s": med([r["refine_s"] for r in rs]), "agg_s": med([r["agg_s"] for r in rs]),
                        "times_s": [r["time_s"] for r in sorted(rs, key=lambda r: r["round"])],
                        "ratio_async": med(ra), "ratio_auto": med(rau), "self_speedup": sp,
                        "modularity": med([r["modularity"] for r in rs]), "modularity_values": sorted(qs),
                        "communities": r0["communities"], "disconnected": r0["disconnected"],
                        "work_edge_passes": work, "subrounds_par": cpar, "subrounds_seq": cseq, "others_busy_cores": [r.get("others_busy_cores") for r in rs],
                        "n_passes": r0["n_passes"], "total_iters": r0["total_iters"], "passes_at_iter_cap": capped,
                        "sub_rounds_per_level": [p.get("sub", 0) for p in r0["passes"]],
                        "hash_levels": sorted({r["hash_levels"] for r in rs}),
                        "loadavg_before": [r["loadavg_before"] for r in rs],
                        "memory_peak": max(r.get("cgroup", {}).get("memory_peak", 0) for r in rs),
                    }
                    out["table"].append(e)
                    qtxt = f"{e['modularity']:.4f}" if len(qs) == 1 or v != "async" else f"{e['modularity']:.4f}~"
                    print(f"{g:6} {it:>2} {v:>10} {t:>3} {len(rs):>2} {e['time_s']:7.3f} {e['lm_s']:6.3f} {e['refine_s']:6.3f} {e['agg_s']:6.3f} "
                          f"{e['ratio_async']:7.2f} {e['ratio_auto']:6.2f} {sp:7.2f} {qtxt:>8} {e['communities']:>6} {e['disconnected']:>4} "
                          f"{e['n_passes']:>6} {e['total_iters']:>5} {capped:>6} {work:5.1f} {str(cpar):>6} {str(cseq):>6} {subs}")
            print()
    if bad:
        print("== Failed rows:", out["failed"])
    if a.seeds:
        # modularity over schedule seeds (--bucket-shift k; k = 4 is the A/B's own run) for each det
        # rule, next to async's run-to-run values (every A/B row of async: all rounds, all threads)
        qs = defaultdict(dict)
        with open(a.seeds) as f:
            for l in f:
                c = json.loads(l)
                qs[(c["graph"], c["iterations"], c["variant"])][int(c["tag"].removeprefix("shift"))] = c["modularity"]
        for (g, it, v), d in list(qs.items()):
            r4 = by.get((g, it, v, 16))
            if r4:
                d.setdefault(4, r4[0]["modularity"])
        for g in graphs:
            for it in its:
                aq = [r["modularity"] for t in ths for r in by.get((g, it, "async", t), [])]
                if aq:
                    qs[(g, it, "async")] = dict(enumerate(aq))
        print("\n== Modularity over schedule seeds (det: --bucket-shift 0-6; async: every A/B run)")
        print(f"{'graph':6} {'it':>2} {'variant':>10} {'n':>3} {'mean':>7} {'min':>7} {'max':>7} {'sd':>7}")
        out["seeds"] = []
        for g in graphs:
            for it in its:
                for v in vs:
                    d = qs.get((g, it, v))
                    if not d:
                        continue
                    x = list(d.values())
                    e = {"graph": g, "iterations": it, "variant": v, "n": len(x), "mean": st.mean(x), "min": min(x),
                         "max": max(x), "sd": st.stdev(x) if len(x) > 1 else 0.0, "values": d}
                    out["seeds"].append(e)
                    print(f"{g:6} {it:>2} {v:>10} {len(x):>3} {e['mean']:.4f} {e['min']:.4f} {e['max']:.4f} {e['sd']:.4f}")
    if a.json:
        with open(a.json, "w") as f:
            json.dump(out, f, indent=1)


if __name__ == "__main__":
    main()
