#!/usr/bin/env python3
"""Is a rec round one independent Leiden per region? (review of PR #162, must-fix 1)

    dev_independence.py STORE WORK GRAPH [ROUND_FLAGS...]

Round 1 of rec (the regions are the communities of T above 64 nodes): one run on the cut graph
of all of them (the union, as rec does it), and one run per chosen region alone, each with the
region's own null model (the same multipliers). For each chosen region: sub-communities in the
union and alone, and the ARI / NMI of the two partitions of its nodes. Regions: T's 1st, 5th,
20th (the reviewer's web-google case) and 100th largest. One JSON line per region.
"""
import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import shape  # noqa: E402

store, work, graph = sys.argv[1:4]
flags = sys.argv[4:]
T = np.fromfile(os.path.join(work, "plain.memb.u32"), np.uint32).astype(np.int64)
u, cnt = np.unique(T, return_counts=True)
rank = u[np.lexsort((u, -cnt))]
big = rank[cnt[np.searchsorted(u, rank)] > 64]


def run(regions, tag, extra=()):
    act = np.isin(T, regions)
    idx = np.nonzero(act)[0]
    lab = np.where(act, T, shape.NONE).astype(np.uint32)
    lf, cut, st = (os.path.join(work, f"ind.{x}") for x in ("lab.u32", "cut", "stats"))
    lab.tofile(lf)
    shape.m102(["cutgraph", "--store", store, "--out", cut, "--labels", lf, "--threads", 16, "--out-stats", st])
    e = np.fromfile(st + ".internal.u64", np.uint64).astype(np.float64)
    gm = e[regions].sum() / e[T[idx]]
    gm.tofile(os.path.join(work, "ind.gmul.f64"))
    mf = os.path.join(work, "ind.memb.u32")
    d = shape.m102(["leiden", "run", "--store", cut, "--config", "comp", "--mem", "mmap", "--threads", 16, "--det", "--gmul", os.path.join(work, "ind.gmul.f64"),
                    "--out-memb", mf, "--no-eval"] + flags + list(extra))
    S = np.full(len(T), -1, np.int64)
    S[idx] = np.fromfile(mf, np.uint32)
    return S, d


U, du = run(big, "union")
for k in (0, 4, 19, 99):
    if k >= len(big):
        continue
    r = big[k]
    A, da = run(np.array([r]), "alone")
    # the noise floor: the same region alone with another visiting order (2048 hashed
    # sub-rounds instead of the automatic 256-1024): what a different visiting order alone does
    B, _ = run(np.array([r]), "alone-order", ("--sub-rounds", "2048"))
    m = T == r
    ari, nmi = shape.ari_nmi(U[m], A[m])
    ari0, nmi0 = shape.ari_nmi(A[m], B[m])
    print(json.dumps({"graph": graph, "round_flags": " ".join(flags), "region_rank": k + 1, "region_nodes": int(m.sum()), "regions_in_union": int(len(big)),
                      "subs_union": int(len(np.unique(U[m]))), "subs_alone": int(len(np.unique(A[m]))), "ari_union_alone": ari, "nmi_union_alone": nmi,
                      "subs_alone_other_order": int(len(np.unique(B[m]))), "ari_alone_other_order": ari0, "nmi_alone_other_order": nmi0,
                      "passes_union": du.get("n_passes"), "passes_alone": da.get("n_passes")}), flush=True)
for x in ("ind.lab.u32", "ind.gmul.f64", "ind.memb.u32", "ind.stats.internal.u64"):
    os.remove(os.path.join(work, x))
