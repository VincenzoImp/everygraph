#!/usr/bin/env python3
"""Draw the top of a gmap layout as SVG, in the style of the lab viewer: regions to a given
depth, found communities solid, aggregates hatched, subdivisions and layout-derived groups
dashed, each labelled with its kind and exact node count when it is large enough. For graphs
whose tile file does not fit the memory limit (Friendster under 16 GiB): it reads only
tnodes.bin.

    svgmap.py LAYOUT_DIR OUT.svg [--depth 3] [--cx 0.5 --cy 0.5 --half 0.5] [--title T]
"""
import argparse
import html

import numpy as np

DT = np.dtype([("parent", "<u4"), ("start", "<u4"), ("end", "<u4"), ("comm", "<u4"), ("level", "i1"), ("depth", "u1"), ("flags", "u1"), ("pad", "u1"),
               ("x", "<f4"), ("y", "<f4"), ("r", "<f4"), ("node_r", "<f4"), ("nkids", "<u4"), ("internal", "<u8"), ("volume", "<u8")])
F_VIRTUAL, F_DEAD, F_AGG, F_SUBDIV = 1, 4, 8, 16

ap = argparse.ArgumentParser()
ap.add_argument("layout")
ap.add_argument("out")
ap.add_argument("--depth", type=int, default=3)
ap.add_argument("--cx", type=float, default=0.5)
ap.add_argument("--cy", type=float, default=0.5)
ap.add_argument("--half", type=float, default=0.5)
ap.add_argument("--title", default="")
ap.add_argument("--size", type=int, default=1000)
a = ap.parse_args()

tn = np.fromfile(a.layout + "/tnodes.bin", dtype=DT)
alive = (tn["flags"] & F_DEAD) == 0
nodes = (tn["end"] - tn["start"]).astype(np.int64)
# top-level region of every tree node, for its colour (rank by size, as the viewer)
top = np.arange(len(tn))
for _ in range(64):
    p = tn["parent"][top]
    nxt = np.where((p != 0) & (top != 0), p, top)
    if (nxt == top).all():
        break
    top = nxt
roots = np.nonzero(alive & (tn["parent"] == 0))[0]
roots = roots[roots != 0]
rank = {int(c): i for i, c in enumerate(sorted(roots, key=lambda c: (-nodes[c], c)))}


def color(r):
    h = ((r * 0.618034) % 1) * 6
    s = 0.55 + 0.15 * (r % 3)
    c, x = s, s * (1 - abs((h % 2) - 1))
    m = 1 - c
    rgb = [[c, x, 0], [x, c, 0], [0, c, x], [0, x, c], [x, 0, c], [c, 0, x]][int(h) % 6]
    return "#%02x%02x%02x" % tuple(int(255 * (v + m)) for v in rgb)


W = a.size
sc = W / (2 * a.half)
X = lambda x: (x - (a.cx - a.half)) * sc
sel = np.nonzero(alive & (tn["depth"] >= 1) & (tn["depth"] <= a.depth))[0]
sel = sel[np.argsort(-tn["r"][sel], kind="stable")]
out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{W + 40}" viewBox="0 0 {W} {W + 40}" style="background:#090a10">',
       '<defs><pattern id="h" width="7" height="7" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><line x1="0" y1="0" x2="0" y2="7" stroke="#ffffff" stroke-opacity="0.28" stroke-width="1.2"/></pattern></defs>',
       f'<text x="10" y="24" fill="#dde" font-family="system-ui" font-size="15" font-weight="600">{html.escape(a.title)}</text>',
       f'<g transform="translate(0,40)">']
labels = []
for i in sel:
    t = tn[i]
    r = float(t["r"]) * sc
    if r < 1.0:
        continue
    x, y = X(float(t["x"])), (float(t["y"]) - (a.cy - a.half)) * sc
    col = color(rank.get(int(top[i]), 0))
    fl = int(t["flags"])
    kind = "aggregate" if fl & F_AGG else "subdivision" if fl & F_SUBDIV else "derived" if fl & F_VIRTUAL else "community"
    dash = ' stroke-dasharray="5,4"' if kind in ("subdivision", "derived") else ""
    fill = 'url(#h)' if kind == "aggregate" else col
    fo = "1" if kind == "aggregate" else "0.08"
    out.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{r:.1f}" fill="{fill}" fill-opacity="{fo}" stroke="{col}" stroke-width="1.3"{dash}/>')
    if r >= 30:
        if kind == "aggregate":
            lines = [f"{int(t['comm']):,} small communities", f"{int(nodes[i]):,} nodes"]
        elif kind == "subdivision":
            lines = ["part (drawing aid)", f"{int(nodes[i]):,} nodes"]
        elif kind == "derived":
            lines = ["derived group (layout)", f"{int(nodes[i]):,} nodes"]
        else:
            lines = [f"{int(nodes[i]):,} nodes"]
        labels.append((x, y - r + 13, lines, kind))
boxes = []
for x, y, lines, kind in labels[:120]:
    w = max(len(s) for s in lines) * 6.4
    if any(abs(bx - x) < (bw + w) / 2 + 4 and abs(by - y) < 13 * max(bl, len(lines)) for bx, by, bw, bl in boxes):
        continue
    boxes.append((x, y, w, len(lines)))
    out.append(f'<rect x="{x - w / 2 - 3:.1f}" y="{y - 10:.1f}" width="{w + 6:.1f}" height="{13 * len(lines)}" fill="#090a10" fill-opacity="0.7"/>')
    for k, s in enumerate(lines):
        c = "#ffffff" if kind == "community" else ("#e8d9a8" if k == 0 else "#c8ccd8")
        out.append(f'<text x="{x:.1f}" y="{y + 13 * k:.1f}" fill="{c}" font-family="system-ui" font-size="11" text-anchor="middle">{html.escape(s)}</text>')
out.append("</g></svg>")
open(a.out, "w").write("\n".join(out))
print(f"{a.out}: {len(sel)} regions to depth {a.depth}, {len(boxes)} labels")
