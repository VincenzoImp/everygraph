#!/usr/bin/env python3
"""m1-12 (#26): a node-level place to look at, for the frame-time runs: the finest community (leaf-parent) with 50 to
80 nodes and the most internal edges. Prints {"x", "y", "nodes", "internal"} in map coordinates.

    target.py LAYOUT_DIR
"""
import json
import sys

import numpy as np

DT = np.dtype([("parent", "<u4"), ("start", "<u4"), ("end", "<u4"), ("comm", "<u4"), ("level", "i1"), ("depth", "u1"), ("flags", "u1"), ("pad", "u1"),
               ("x", "<f4"), ("y", "<f4"), ("r", "<f4"), ("node_r", "<f4"), ("nkids", "<u4"), ("internal", "<u8"), ("volume", "<u8")])
tn = np.fromfile(sys.argv[1] + "/tnodes.bin", dtype=DT)
n = tn["end"].astype(np.int64) - tn["start"].astype(np.int64)
ok = np.nonzero(((tn["flags"] & 2) != 0) & ((tn["flags"] & 4) == 0) & (n >= 50) & (n <= 80))[0]
t = ok[np.argmax(tn["internal"][ok])]
print(json.dumps({"x": float(tn["x"][t]), "y": float(tn["y"][t]), "nodes": int(n[t]), "internal": int(tn["internal"][t])}))
