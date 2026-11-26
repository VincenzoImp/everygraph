#!/usr/bin/env python3
"""Issue #172 (m1-15): tables and A/B ratios from rows of the `mmap-hints` spec.

    python3 -I summarize.py ROWS.jsonl [ROWS.jsonl ...]

For each benchmark, one line per row (in run order) with its matrix values and metrics,
then, for cases of one harness run that differ only in `variant` within the same `pass`
(and limit / prefetch), the ratio of every metric against variant `none` (pq: random=0),
and the median of those per-pass ratios.
Bytes in GB (10^9), memory in MiB, refaults and faults in millions.
"""
import json
import statistics
import sys
from collections import defaultdict

FMT = {
    "B": lambda v: f"{v / 1e9:.2f} GB",
    "pages": lambda v: f"{v / 1e6:.1f} M",
}
MIB = {"pagetables_peak", "kernel_peak", "shmem_peak", "anon_peak_sampled"}
SMALL_BYTES = {"neighbors_bytes", "degree_bytes", "common_bytes", "two_hop_bytes", "khop3_bytes",
               "bibfs_bytes", "tput_neighbors_bytes"}


def fmt(name, v, unit):
    if v is None:
        return "-"
    if name in MIB:
        return f"{v / 2**20:.0f} MiB"
    if name in SMALL_BYTES:
        return f"{v / 1024:.1f} KiB"
    if unit in FMT:
        return FMT[unit](v)
    if name == "major_faults":
        return f"{v / 1e6:.2f} M"
    if isinstance(v, float):
        return f"{v:.4g}"
    return str(v)


def main():
    rows = []
    for p in sys.argv[1:]:
        for line in open(p):
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    by_bench = defaultdict(list)
    for r in rows:
        by_bench[r["benchmark"]].append(r)
    for b, rs in by_bench.items():
        print(f"## {b}")
        for r in rs:
            v = {k: x for k, x in (r.get("vars") or {}).items()}
            m = r.get("metrics") or {}
            units = r.get("units") or {}
            dist = (r.get("disturbance") or {}).get("verdict")
            others = ((r.get("disturbance") or {}).get("others") or {})
            rec = r.get("recorded") or {}
            head = " ".join(f"{k}={v[k]}" for k in sorted(v)) + f" [{r['status']}]"
            body = ", ".join(f"{k} {fmt(k, x, units.get(k))}" for k, x in m.items())
            io = r.get("io") or {}
            extra = f"; harness read {io.get('read_bytes', 0) / 1e9:.2f} GB; load {r.get('load_start')}"
            if rec.get("pos_sha256"):
                extra += f"; pos {rec['pos_sha256'][:12]} tnodes {rec.get('tnodes_sha256', '')[:12]}"
            print(f"- {head}: {body}{extra}")
        # A/B ratios: group by everything except variant
        groups = defaultdict(dict)
        for r in rs:
            if r["status"] != "ok":
                continue
            v = dict(r.get("vars") or {})
            var = v.pop("variant", None)
            v["run"] = r.get("run_id", "")
            key = tuple(sorted(v.items()))
            groups[key][var] = r.get("metrics") or {}
        ratios = defaultdict(lambda: defaultdict(list))
        for key, d in groups.items():
            if len(d) < 2:
                continue
            base = "none" if "none" in d else sorted(d)[0]  # "0" for pq's random=0
            a = d[base]
            k_nopass = tuple(x for x in key if x[0] != "pass")
            for var, bm in d.items():
                if var == base:
                    continue
                for name in a:
                    if isinstance(a[name], (int, float)) and isinstance(bm.get(name), (int, float)) and a[name]:
                        ratios[(k_nopass, var, base)][name].append(bm[name] / a[name])
        for (k, var, base), mm in sorted(ratios.items()):
            label = " ".join(f"{x}={y}" for x, y in k) or "all"
            print(f"  ratios {label}, {var} / {base} (median of {len(next(iter(mm.values())))} passes):")
            for name, xs in mm.items():
                print(f"    {name}: {statistics.median(xs):.3f} ({', '.join(f'{x:.3f}' for x in xs)})")
        print()


if __name__ == "__main__":
    main()
