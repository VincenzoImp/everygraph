"""Markdown tables from results/*.jsonl (used to build REPORT.md)."""
import json, os, sys, collections
R = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "results")
def rows(name):
    p = os.path.join(R, name)
    if not os.path.exists(p): return []
    return [json.loads(l) for l in open(p) if l.strip().startswith("{")]

def ingest_table(files, pred=lambda r: True):
    print("| run | threads | budget GB | total s | text GB/s | M edges/s | peak RSS GB | temp disk GB | parse s | relabel+partition s | sort+dedup+write s | load avg |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for f in files:
        for r in rows(f):
            if r.get("cmd") != "ingest" or "phase_seconds" not in r or not pred(r): continue
            ph = r["phase_seconds"]
            print(f"| {r.get('label','')} | {r['threads']} | {r['mem_budget_gb']:g} | {r['seconds']:.2f} | {r['text_gb_per_s']:.2f} | {r['edges_per_s']/1e6:.1f} | {r['peak_rss_gb']:.2f} | {r['temp_disk_gb']:.1f} | {ph['pass1_parse']:.2f} | {ph['pass2_relabel_partition']:.2f} | {ph['pass3_sort_dedup_write']:.2f} | {r['loadavg_1m_at_end']:.0f} |")

def kernel_table(files, graph, pred=lambda r: True):
    best = collections.OrderedDict()
    for f in files:
        for r in rows(f):
            if r.get("cmd") != "run" or r["graph"] != graph or not pred(r): continue
            it = r["result"].get("iters", 1) if r["algo"].startswith("pr") else 1
            k = (r["algo"], r["threads"])
            v = r["seconds_best"] / it if r["algo"].startswith("pr") else r["seconds_best"]
            if k not in best or v < best[k]: best[k] = v
    algos = []; ths = set()
    for (a, t) in best:
        if a not in algos: algos.append(a)
        ths.add(t)
    ths = sorted(ths)
    print("| algo | " + " | ".join(f"T={t}" for t in ths) + " | speedup min->max T |")
    print("|---|" + "---|" * (len(ths) + 1))
    for a in algos:
        cells = [f"{best[(a,t)]:.3f}" if (a, t) in best else "" for t in ths]
        have = [t for t in ths if (a, t) in best]
        sp = f"{best[(a,have[0])]/best[(a,have[-1])]:.1f}x ({have[0]}->{have[-1]})" if len(have) > 1 else ""
        unit = " (s/iter)" if a.startswith("pr") else " (s)"
        print(f"| {a}{unit} | " + " | ".join(cells) + f" | {sp} |")

def probe_table(files):
    print("| label | algo | T | load | THP | ids | order | best s (20 it) | all | load avg |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for f in files:
        for r in rows(f):
            if r.get("cmd") != "run": continue
            print(f"| {r.get('label','')} | {r['algo']} | {r['threads']} | {r['load']} | {r.get('thp','default')} | {r['ids']} | {r['order']} | {r['seconds_best']:.3f} | {' '.join('%.2f'%x for x in r['seconds_all'])} | {r['loadavg_1m_at_end']:.0f} |")

def ooc_table(files):
    print("| label | algo | I/O | T | total s | s/iter (first; mean of rest) | read GB/s | peak RSS GB | anon GB | exit |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for f in files:
        rs = rows(f); exits = {r["label"]: r for r in rs if r.get("cmd") == "ooc-exit"}
        seen = set()
        for r in rs:
            lab = r.get("label", "")
            if r.get("cmd") == "semiext":
                it = r["iter_seconds"]; per = f"{it[0]:.2f}; {sum(it[1:])/max(1,len(it)-1):.2f}" if len(it) > 1 else ""
                ex = exits.get(lab, {}).get("exit_code", ""); seen.add(lab)
                print(f"| {lab} | {r['algo']} | {r['io']} | {r['threads']} | {r['seconds']:.1f} | {per} | {r['read_gb_per_s_wall']:.2f} | {r['peak_rss_gb']:.2f} | {r['rss_anon_gb']:.2f} | {ex} |")
            elif r.get("cmd") == "run":
                it = r["result"].get("iters"); per = f"mean {r['seconds_best']/it:.2f}" if it else ""
                ex = exits.get(lab, {}).get("exit_code", ""); seen.add(lab)
                print(f"| {lab} | {r['algo']} | mmap ({r['load']}) | {r['threads']} | {r['seconds_best']:.1f} | {per} | | {r['peak_rss_gb']:.2f} | {r['rss_anon_gb']:.2f} | {ex} |")
        for lab, e in exits.items():
            if lab not in seen:
                print(f"| {lab} | | | | >{e.get('timeout_s','')} (no result) | | | | | {e['exit_code']} |")

if __name__ == "__main__":
    what = sys.argv[1]
    if what == "ingest": ingest_table(sys.argv[2:])
    elif what == "kernels": kernel_table(sys.argv[3:], sys.argv[2])
    elif what == "probes": probe_table(sys.argv[2:])
    elif what == "ooc": ooc_table(sys.argv[2:])
