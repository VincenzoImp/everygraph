#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Tables of the m1-09 bake-off from results/*.jsonl: results/summary.txt and summary.json.

The last row of a (graph, order) pair wins, so a re-run replaces an earlier one."""
import json
import pathlib

R = pathlib.Path(__file__).resolve().parent / "results"
GRAPHS = ["web-google", "lj", "orkut", "twitter-sym", "friendster"]
ORDERS = ["identity", "random", "dbg", "bfs", "flat", "hier", "hier-deg", "hier-bfs", "hier-cap64",
          "llp", "bp", "bp-input", "bp-nocool", "bp-self", "bp-full"]


def rows(name):
    p = R / name
    return [json.loads(l) for l in p.read_text().splitlines() if l.strip()] if p.exists() else []


def last(rs, key):
    out = {}
    for r in rs:
        out[key(r)] = r
    return out


ev = last(rows("eval.jsonl"), lambda r: (r["graph"], r["order"]))
od = last(rows("orders.jsonl"), lambda r: (r["graph"], r["name"]))
bv = last(rows("bv.jsonl"), lambda r: (r["graph"], r["order"]))
leiden = {p.stem: json.loads(p.read_text()) for p in (R / "leiden").glob("*.json")}


def ratio(e, source, kernel):
    for k in e["kernels"]:
        if k["source"] == source and k["kernel"] == kernel:
            return k["ratio_b_over_a"], k["ok"]
    return None, True


def fmt(x, f="{:.2f}"):
    return "—" if x is None else f.format(x)


lines, summary = [], []
for g in GRAPHS:
    base = ev.get((g, "identity"))
    if not base:
        continue
    lines.append(f"\n## {g}: n = {base['n']:,}, entries = {base['entries']:,}, "
                 f"random-list bound 2 + log2(n/d) = {base['random_bound']:.2f} bits\n")
    sources = sorted({k["source"] for e in ev.values() if e["graph"] == g for k in e["kernels"]})
    head = ["order", "order s", "perm s", "enc s", "bits/e", "vs orig", "loggap", "ratio", "BV b/arc", "0124 est"]
    for s in sources:
        head += [f"PR {s}", f"BFS {s}", f"WCC {s}"]
    lines.append(" | ".join(head))
    lines.append(" | ".join("---" for _ in head))
    for o in ORDERS:
        e = ev.get((g, o))
        if not e:
            continue
        t = od.get((g, o), {}).get("order_s")
        b = bv.get((g, o), {}).get("bv_bits_per_arc")
        row = [o, fmt(t, "{:.1f}"), fmt(e["permute_s"]), fmt(e["encode_s"]), fmt(e["bits_per_entry"]),
               f"{100 * (e['bits_per_entry'] / base['bits_per_entry'] - 1):+.1f}%", fmt(e["loggap"]),
               fmt(e["locality_ratio"], "{:.3f}"), fmt(b), fmt(e["svb0124_data_bits_est"])]
        rec = {k: e[k] for k in ("graph", "order", "bits_per_entry", "data_bits_per_entry", "loggap",
                                  "locality_ratio", "permute_s", "encode_s", "svb0124_data_bits_est")}
        rec.update(order_s=t, bv_bits_per_arc=b)
        bad = []
        for s in sources:
            for k in ("pr", "bfs", "wcc"):
                x, ok = ratio(e, s, k)
                row.append(fmt(x))
                rec[f"{k}_{s}_ratio"] = x
                if not ok:
                    bad.append(f"{k}/{s}")
        if bad:
            row[0] += " (CHECK FAILED: " + ",".join(bad) + ")"
        lines.append(" | ".join(row))
        summary.append(rec)
    for v in ("natural", "cap64"):
        L = leiden.get(f"{g}-{v}")
        if L:
            lines.append(f"\nLeiden {v} (lab b2, 16 threads): {L['time_s']:.1f} s, Q = {L['modularity']:.4f}, "
                         f"levels {L['level_sizes']}")

lines.append("\n## Locality score of the input order against the best gain measured\n")
lines.append("graph | ratio (input ids) | loggap (input) | best order | bits/e best | gain vs input | random vs input")
lines.append("--- | --- | --- | --- | --- | --- | ---")
for g in GRAPHS:
    base = ev.get((g, "identity"))
    if not base:
        continue
    cands = [e for (gg, o), e in ev.items() if gg == g and o != "identity"]
    best = min(cands, key=lambda e: e["bits_per_entry"]) if cands else base
    rnd = ev.get((g, "random"))
    lines.append(f"{g} | {base['locality_ratio']:.3f} | {base['loggap']:.2f} | {best['order']} | "
                 f"{best['bits_per_entry']:.2f} | {100 * (best['bits_per_entry'] / base['bits_per_entry'] - 1):+.1f}% | "
                 + (f"{100 * (rnd['bits_per_entry'] / base['bits_per_entry'] - 1):+.1f}%" if rnd else "—"))

lo = rows("leiden_orders.jsonl")
if lo:
    import statistics as st
    groups = {}
    for r in lo:
        groups.setdefault((r["graph"], r["mode"], r["visit"], r["order"]), []).append(r)
    lines.append("\n## Leiden on the graph stored in each order (#146's fifth arm, in RAM, lab b2, 16 threads)\n")
    lines.append("graph | mode | visit | stored order | Q (each run) | communities | median s")
    lines.append("--- | --- | --- | --- | --- | --- | ---")
    for k in sorted(groups):
        v = groups[k]
        lines.append(f"{k[0]} | {k[1]} | {k[2]} | {k[3]} | {', '.join(f'{x['modularity']:.4f}' for x in v)} | "
                     f"{', '.join(str(x['communities']) for x in v)} | {st.median(x['time_s'] for x in v):.2f}")

text = ("Exploratory (shared machine). Kernel columns: median time on the reordered graph / median "
        "on the original ids, interleaved in one process, 16 threads, proof kernels (PageRank f32 "
        "20 iterations, direction-optimizing BFS from the largest-degree node, Afforest WCC). "
        "bits/e: codec 0, index included. loggap: mean log2 gap per entry (first gap |x0 - v|). "
        "ratio: bits/e over the random-list bound. 0124 est: Stream VByte 0124 data bits per entry, "
        "estimated from word sizes (no index).\n" + "\n".join(lines) + "\n")
(R / "summary.txt").write_text(text)
(R / "summary.json").write_text(json.dumps(summary, indent=1) + "\n")
print(text)
