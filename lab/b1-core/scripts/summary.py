"""Print a compact table from results/bench/*.json (or any dir with the same files)."""
import json, sys, os
d = sys.argv[1]
def load(n):
    p = os.path.join(d, n)
    return json.load(open(p)) if os.path.exists(p) else []
print("-- ingest")
for r in load("ingest.json"):
    if "phase_seconds" in r:
        ph = r["phase_seconds"]
        print(f"{r.get('label',''):34s} T={r['threads']:<2d} {r['seconds']:8.2f}s {r['text_gb_per_s']:5.2f} GB/s {r['edges_per_s']/1e6:6.1f} Medges/s rss={r['peak_rss_gb']:.2f}GB tmp={r['temp_disk_gb']:.1f}GB "
              f"p1={ph['pass1_parse']:.2f} p2={ph['pass2_relabel_partition']:.2f} p3={ph['pass3_sort_dedup_write']:.2f}")
    elif r.get("parse_only"):
        print(f"{r.get('label',''):34s} T={r['threads']:<2d} {r['seconds']:8.2f}s {r['text_gb_per_s']:5.2f} GB/s (parse only)")
print("-- kernels")
for r in load("kernels.json"):
    if r.get("cmd") == "run":
        print(f"{r.get('label',''):16s} {r['graph']:11s} {r['algo']:5s} T={r['threads']:<2d} load={r['load']:10s} thp={r.get('thp','default'):7s} ids={r['ids']} order={r['order']:6s} best={r['seconds_best']:8.3f}s")
print("-- semi-external / out-of-core")
for r in load("semiext.json"):
    if r.get("cmd") == "semiext":
        it = r["iter_seconds"]
        per = (sum(it[1:]) / max(1, len(it) - 1)) if len(it) > 1 else float("nan")
        print(f"{r.get('label',''):28s} {r['algo']:5s} io={r['io']:9s} total={r['seconds']:7.2f}s per-iter(after 1st)={per:6.2f}s read={r['read_gb_per_s_wall']:.2f} GB/s rss={r['peak_rss_gb']:.2f}GB")
    elif r.get("cmd") == "run":
        print(f"{r.get('label',''):28s} {r['algo']:5s} mmap      total={r['seconds_best']:7.2f}s iters={r['result'].get('iters')} rss={r['peak_rss_gb']:.2f}GB")
    elif r.get("cmd") == "ooc-exit":
        print(f"   exit {r['label']} limit={r['limit']} code={r['exit_code']}")
