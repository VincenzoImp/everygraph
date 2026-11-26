# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Markdown tables from the probe's JSON lines: per engine, the median kIOPS over the
repetitions with the min-max spread; MB/s; and the pread pool's IOPS over the best
io_uring engine's, per repetition (median and spread)."""
import json
import statistics as st
import sys

rows = [json.loads(l) for l in open(sys.argv[1])]
idle = [r for r in rows if r["kind"] == "idle"][0]
cells = [r for r in rows if r["kind"] == "cell"]
print(f"idle device utilisation over 10 s before the run: {idle['dev_util']:.3f}, "
      f"load1 {idle['load1']}, file pages resident at start {idle['resident_pages_at_start']}"
      f" of {idle['file_pages']}")
by = {(r["size_kib"], r["qd"], r["engine"], r["mode"], r["rep"]): r for r in cells}
sizes = sorted({r["size_kib"] for r in cells})
qds = sorted({r["qd"] for r in cells})
reps = sorted({r["rep"] for r in cells})
engines = list(dict.fromkeys(r["engine"] for r in sorted(cells, key=lambda r: r["engine"] != "pread")))
urings = [e for e in engines if e != "pread"]
extra = sum(r["dev_read_bytes"] for r in cells) / sum(r["reads"] * r["size_kib"] * 1024 for r in cells)
print(f"device read bytes / requested bytes over all cells: {extra:.4f}")
foreign = sorted(1 - r["reads"] / max(r["dev_reads"], 1) for r in cells)
print(f"share of device reads not ours per cell: median {st.median(foreign):.4f}, max {foreign[-1]:.4f}")
print(f"capped cells (offset list ran out before the time limit): {sum(r['capped'] for r in cells)}")
for mode in ("direct", "buffered"):
    print(f"\n### {mode}\n")
    head = ["size", "QD"] + [f"{e} kIOPS" for e in engines] + [
        "pread MB/s", "pread / best io_uring", "pread p50/p99 us", "device util"]
    print("| " + " | ".join(head) + " |")
    print("|" + "---|" * len(head))
    for s in sizes:
        for q in qds:
            v = {e: [by[(s, q, e, mode, rp)] for rp in reps if (s, q, e, mode, rp) in by] for e in engines}
            if not v["pread"]:
                continue
            out = [f"{s} KiB", str(q)]
            for e in engines:
                x = [r["iops"] / 1e3 for r in v[e]]
                out.append(f"{st.median(x):.1f} ({min(x):.1f}–{max(x):.1f})")
            out.append(f"{st.median(r['bytes_per_s'] / 1e6 for r in v['pread']):.0f}")
            best = max(urings, key=lambda e: st.median(r["iops"] for r in v[e]))
            ratio = [a["iops"] / b["iops"] for a, b in zip(v["pread"], v[best])]
            out.append(f"{st.median(ratio):.2f} ({min(ratio):.2f}–{max(ratio):.2f}) vs {best}")
            out.append(f"{st.median(r['lat_p50_us'] for r in v['pread']):.0f}/"
                       f"{st.median(r['lat_p99_us'] for r in v['pread']):.0f}")
            out.append(f"{st.median(r['dev_util'] for e in engines for r in v[e]):.2f}")
            print("| " + " | ".join(out) + " |")
