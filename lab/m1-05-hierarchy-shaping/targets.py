#!/usr/bin/env python3
"""Pick deterministic screenshot targets from a gmap layout, and count its kinds of region.

    targets.py LAYOUT_DIR > targets.json

Targets: the overview; the largest aggregate under the root (if any); the largest top-level
community; the largest child of that community; a leaf community of 20-120 nodes inside it
(node level). Zoom z is chosen so that the target's disc is about 380 px across in the lab
viewer (scale = 256 * 2^z px per map unit).
"""
import json
import math
import sys

import numpy as np

DT = np.dtype([("parent", "<u4"), ("start", "<u4"), ("end", "<u4"), ("comm", "<u4"), ("level", "i1"), ("depth", "u1"), ("flags", "u1"), ("pad", "u1"),
               ("x", "<f4"), ("y", "<f4"), ("r", "<f4"), ("node_r", "<f4"), ("nkids", "<u4"), ("internal", "<u8"), ("volume", "<u8")])
F_VIRTUAL, F_LEAFP, F_DEAD, F_AGG, F_SUBDIV = 1, 2, 4, 8, 16

lay = sys.argv[1]
tn = np.fromfile(lay + "/tnodes.bin", dtype=DT)
alive = (tn["flags"] & F_DEAD) == 0
size = (tn["end"] - tn["start"]).astype(np.int64)
n = int(size[0])
fl = tn["flags"]


def zoom_for(r, px=380.0):
    return round(math.log2(px / (2 * 256 * float(r))), 2)


def tgt(name, i):
    return {"name": name, "tnode": int(i), "x": float(tn["x"][i]), "y": float(tn["y"][i]), "z": zoom_for(tn["r"][i]), "nodes": int(size[i]),
            "kind": "aggregate" if fl[i] & F_AGG else "subdivision" if fl[i] & F_SUBDIV else "derived" if fl[i] & F_VIRTUAL else "community"}


kids = {}
for i in np.nonzero(alive)[0][1:]:
    kids.setdefault(int(tn["parent"][i]), []).append(int(i))
out = [{"name": "1-overview", "x": 0.5, "y": 0.5, "z": 1.6}]
root_kids = kids.get(0, [])
aggs = [i for i in root_kids if fl[i] & F_AGG]
if aggs:
    out.append(tgt("2-aggregate", max(aggs, key=lambda i: (size[i], -i))))
comms = [i for i in root_kids if not fl[i] & F_AGG]
big = max(comms, key=lambda i: (size[i], -i))
out.append(tgt("3-largest-community", big))
sub = [i for i in kids.get(big, []) if not fl[i] & F_AGG]
if sub:
    s = max(sub, key=lambda i: (size[i], -i))
    out.append(tgt("4-inside-largest", s))
    # a leaf community of readable size somewhere under s
    stack, leaves = [s], []
    while stack:
        v = stack.pop()
        ch = kids.get(v, [])
        if not ch and 20 <= size[v] <= 120:
            leaves.append(v)
        stack.extend(ch)
    if leaves:
        out.append(tgt("5-node-level", sorted(leaves)[len(leaves) // 2]))
counts = {
    "tnodes_alive": int(alive.sum()),
    "derived_by_layout": int((alive & ((fl & F_VIRTUAL) != 0)).sum()),
    "nodes_under_derived_by_layout_leafparents": int(size[alive & ((fl & F_VIRTUAL) != 0) & ((fl & F_LEAFP) != 0)].sum()),
    "aggregates": int((alive & ((fl & F_AGG) != 0)).sum()),
    "subdivisions": int((alive & ((fl & F_SUBDIV) != 0)).sum()),
    "max_children": int(max((len(v) for v in kids.values()), default=0)),
    "max_depth": int(tn["depth"][alive].max()),
}
json.dump({"targets": out, "counts": counts}, sys.stdout, indent=1)
