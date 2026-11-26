#!/usr/bin/env python3
"""m1-06 (#20): the connectivity overview that needs no selection, measured on a lab map.

For every edge it finds the parent in the map tree where the edge stops being internal (the
lowest common ancestor of its two ends) and the two children of that parent it joins. Summed,
this gives the exact sibling matrix of every parent: what a "reordered matrix of the current
parent's children" would show and what a per-parent section of the map file would hold.

    overview.py CSR_DIR LAYOUT_DIR NAME --json results/NAME.json [--preview OUT.json]

Checks (oracle): for every tree node P, internal(P) computed by gmap equals the sum of its tree
children's internal edges plus the edges this script assigns to P. The top level is compared
with the hierarchy's own quotient graph when the hierarchy directory is given (--hier).
Load-independent counts only; no timings.
"""
import argparse
import json
import zlib

import numpy as np

DT = np.dtype([("parent", "<u4"), ("start", "<u4"), ("end", "<u4"), ("comm", "<u4"), ("level", "i1"), ("depth", "u1"), ("flags", "u1"), ("pad", "u1"),
               ("x", "<f4"), ("y", "<f4"), ("r", "<f4"), ("node_r", "<f4"), ("nkids", "<u4"), ("internal", "<u8"), ("volume", "<u8")])
F_VIRTUAL, F_LEAFP, F_DEAD, F_AGG, F_SUBDIV, F_AGG_INNER = 1, 2, 4, 8, 16, 32
BANDS = (6, 12, 20, 50)

ap = argparse.ArgumentParser()
ap.add_argument("csr")
ap.add_argument("layout")
ap.add_argument("name")
ap.add_argument("--json", required=True)
ap.add_argument("--hier")
ap.add_argument("--preview")
ap.add_argument("--preview-parents", type=int, default=6, help="largest top-level regions whose own matrix goes into the preview")
ap.add_argument("--chunk", type=int, default=4_000_000, help="edges per chunk")
a = ap.parse_args()

meta = json.load(open(a.csr + "/meta.json"))
off = np.fromfile(a.csr + "/offsets.u64", dtype=np.uint64).astype(np.int64)
tgt = np.memmap(a.csr + "/targets.u32", dtype=np.uint32, mode="r")
tn = np.fromfile(a.layout + "/tnodes.bin", dtype=DT)
leaf = np.fromfile(a.layout + "/leafparent.u32", dtype=np.uint32).astype(np.int64)
n, NT = len(off) - 1, len(tn)
alive = (tn["flags"] & F_DEAD) == 0
depth = tn["depth"].astype(np.int64)
par = tn["parent"].astype(np.int64)
nodes = (tn["end"].astype(np.int64) - tn["start"].astype(np.int64))
D = int(depth[alive].max())

# anc[d, u]: u's ancestor at tree depth d; NT + u (the node itself) below its deepest tree node
anc = np.empty((D + 2, n), dtype=np.int64)
anc[:] = NT + np.arange(n, dtype=np.int64)
cur = leaf.copy()
for d in range(D, -1, -1):
    m = depth[cur] == d
    anc[d, m] = cur[m]
    if d > 0:
        cur[m] = par[cur[m]]
assert (anc[0] == 0).all(), "every node must hang under the root (tree node 0)"

lca_w = np.zeros(NT, dtype=np.int64)          # edges whose two ends first split under this parent
loops = 0
keys_parts, w_parts = [], []                  # region pairs (both ends tree nodes): key a * NT + b, a < b
node_pairs = 0                                # pairs where at least one end is a graph node (leaf-parent level)
lo = 0
while lo < n:
    hi = int(np.searchsorted(off, off[lo] + a.chunk, side="right")) - 1
    hi = max(hi, lo + 1)
    hi = min(hi, n)
    deg = off[lo + 1:hi + 1] - off[lo:hi]
    src = np.repeat(np.arange(lo, hi, dtype=np.int64), deg)
    dst = np.asarray(tgt[off[lo]:off[hi]], dtype=np.int64)
    loops += int((src == dst).sum())
    keep = src < dst
    src, dst = src[keep], dst[keep]
    diff = anc[:, src] != anc[:, dst]
    ds = diff.argmax(axis=0)
    cols = np.arange(len(src))
    ca, cb = anc[ds, src], anc[ds, dst]
    lca = anc[ds - 1, src]
    lca_w += np.bincount(lca, minlength=NT)
    reg = (ca < NT) & (cb < NT)
    node_pairs += int((~reg).sum())
    x, y = np.minimum(ca[reg], cb[reg]), np.maximum(ca[reg], cb[reg])
    k, c = np.unique(x * NT + y, return_counts=True)
    keys_parts.append(k)
    w_parts.append(c)
    lo = hi
keys = np.concatenate(keys_parts)
ws = np.concatenate(w_parts)
order = np.argsort(keys, kind="stable")
keys, ws = keys[order], ws[order]
first = np.r_[True, keys[1:] != keys[:-1]]
idx = np.cumsum(first) - 1
pk = keys[first]
pw = np.bincount(idx, weights=ws).astype(np.int64)
pa, pb = pk // NT, pk % NT
pp = par[pa]
assert (par[pb] == pp).all(), "both ends of a region pair must be siblings"

# oracle: internal(P) = sum of tree children's internal + edges split at P
child_int = np.bincount(par[1:][alive[1:]], weights=tn["internal"][1:][alive[1:]].astype(np.float64), minlength=NT).astype(np.int64)
check_nodes = np.nonzero(alive)[0]
mism = check_nodes[(tn["internal"][check_nodes].astype(np.int64) != child_int[check_nodes] + lca_w[check_nodes])]
m_und = int(meta["m_undirected"])

# per-parent matrices of region children
kids_of = {}
for c in np.nonzero(alive & (np.arange(NT) != 0))[0]:
    kids_of.setdefault(int(par[c]), []).append(int(c))
pair_order = np.argsort(pp, kind="stable")
pp_s, pa_s, pb_s, pw_s = pp[pair_order], pa[pair_order], pb[pair_order], pw[pair_order]
bounds = np.searchsorted(pp_s, np.arange(NT + 1))

vol = tn["volume"].astype(np.float64)
# a child's index in its parent's matrix: children by size, largest first
rank = np.zeros(NT, dtype=np.int64)
for p_, ks in kids_of.items():
    ks.sort(key=lambda c: (-nodes[c], c))
    rank[ks] = np.arange(len(ks))


def matrix(p):
    kids = kids_of.get(p, [])
    s, e = bounds[p], bounds[p + 1]
    I = rank[pa_s[s:e]]
    J = rank[pb_s[s:e]]
    W = pw_s[s:e].astype(np.int64)
    sw = I > J
    I[sw], J[sw] = J[sw], I[sw]
    return kids, I, J, W


def block_bytes(I, J, W):
    rec = np.zeros(len(I), dtype=[("i", "<u2"), ("j", "<u2"), ("w", "<u4")])
    o = np.lexsort((J, I))
    rec["i"], rec["j"], rec["w"] = I[o], J[o], W[o]
    raw = rec.tobytes()
    return len(raw), len(zlib.compress(raw, 6))


def seriation(k, I, J, W, kids, how):
    """A permutation of 0..k-1 (position -> child index)."""
    if how == "size":
        return np.arange(k)
    if how == "angle":
        p = int(par[kids[0]])
        ang = np.arctan2(tn["y"][kids] - tn["y"][p], tn["x"][kids] - tn["x"][p])
        return np.lexsort((np.arange(k), ang))
    A = np.zeros((k, k))
    if how == "spectral":
        A[I, J] = W
    else:  # spectral on enrichment, degree-corrected (the modularity null model)
        A[I, J] = W / np.maximum(vol[np.array(kids)][I] * vol[np.array(kids)][J] / (2.0 * m_und), 1e-12)
    A = A + A.T
    d = A.sum(1)
    d[d == 0] = 1.0
    L = np.eye(k) - A / np.sqrt(np.outer(d, d))
    val, vec = np.linalg.eigh(L)
    f = vec[:, 1] / np.sqrt(d)
    if f[np.argmax(np.abs(f))] < 0:
        f = -f
    return np.lexsort((np.arange(k), f))


def arrangement(perm, I, J, W, k):
    """Weighted linear arrangement against the mean of 64 random orders (lower is better), and
    the share of the weight within max(2, k/10) cells of the diagonal (higher is better)."""
    pos = np.empty(k, dtype=np.int64)
    pos[perm] = np.arange(k)
    dist = np.abs(pos[I] - pos[J])
    cost = float((W * dist).sum())
    rng = np.random.default_rng(1)
    rand = []
    for _ in range(64):
        q = rng.permutation(k)
        rand.append(float((W * np.abs(q[I] - q[J])).sum()))
    near = float(W[dist <= max(2, k // 10)].sum()) / max(float(W.sum()), 1.0)
    return cost / np.mean(rand), near


def coupling(kids, I, J, W):
    """Edges between two children over the number expected if the edges were wired at random
    with the same degrees (the modularity null model): what the matrix shades."""
    v = vol[np.array(kids)]
    return W * (2.0 * m_und) / (v[I] * v[J])


def parent_stats(p, with_order=False):
    kids, I, J, W = matrix(p)
    k = len(kids)
    tot = float(W.sum())
    inside = float(tn["internal"][kids].astype(np.float64).sum())
    out = {"parent": p, "k": k, "pairs_nonzero": int(len(W)), "density": len(W) / max(k * (k - 1) / 2, 1),
           "weight_between_children": int(tot), "weight_inside_children": int(inside),
           "share_inside_children": inside / max(inside + tot, 1.0)}
    ws_sorted = np.sort(W)[::-1]
    out["bands_by_weight_share"] = {str(b): float(ws_sorted[:b].sum()) / max(tot, 1.0) for b in BANDS}
    top12 = np.argsort(-W, kind="stable")[:12]
    out["bands12_touching_largest_child"] = int(((I[top12] == 0) | (J[top12] == 0)).sum())
    out["bands12_distinct_children"] = int(len(set(I[top12].tolist()) | set(J[top12].tolist())))
    if len(W):
        exp = vol[np.array(kids)][I] * vol[np.array(kids)][J] / (2.0 * m_und)
        enr = W / exp
        out["pairs_above_expected"] = float((enr > 1).mean())
        out["weight_in_pairs_above_expected"] = float(W[enr > 1].sum()) / tot
        # "which regions are most tightly coupled": the 10 most enriched pairs (pairs at or above the
        # median weight, so that two tiny regions with one edge do not win) against the 12 bands
        cand = np.nonzero(W >= np.median(W))[0]
        top_enr = set(cand[np.argsort(-enr[cand], kind="stable")[:10]].tolist())
        top_w = set(np.argsort(-W, kind="stable")[:12].tolist())
        out["top10_enriched_among_12_heaviest"] = len(top_enr & top_w)
    if with_order and k >= 4:
        out["ordering"] = {}
        for how in ("size", "angle", "spectral", "spectral_enrichment"):
            perm = seriation(k, I, J, W, kids, how)
            ratio, near = arrangement(perm, I, J, W, k)
            cr, cn = arrangement(perm, I, J, coupling(kids, I, J, W), k)
            out["ordering"][how] = {"weight": {"linear_arrangement_vs_random": ratio, "near_diagonal": near},
                                    "coupling": {"linear_arrangement_vs_random": cr, "near_diagonal": cn}}
    return out, kids, I, J, W


res = {"graph": a.name, "n": n, "m": m_und, "self_loops_in_csr": loops // 1, "tree_nodes": int(NT), "tree_nodes_alive": int(alive.sum()), "max_depth": D,
       "oracle": {"tree_nodes_checked": int(len(check_nodes)), "internal_mismatches": int(len(mism)), "first_mismatches": mism[:5].tolist(),
                  "edges_assigned": int(lca_w.sum()), "edges_expected": m_und - loops // 2}}

# the whole tree: what a per-parent matrix section would cost
region_parents = [p for p in range(NT) if bounds[p + 1] > bounds[p]]
raw_tot, gz_tot, gz_per = 0, 0, []
k_hist = []
for p in region_parents:
    kids, I, J, W = matrix(p)
    r_, g_ = block_bytes(I, J, W)
    raw_tot += r_ + 4
    gz_tot += g_
    gz_per.append(g_)
    k_hist.append(len(kids))
gz_per = np.array(gz_per)
res["matrix_section"] = {
    "parents_with_region_pairs": len(region_parents), "region_pairs": int(len(pw)), "region_pair_weight": int(pw.sum()),
    "node_level_pairs": node_pairs, "raw_bytes_8B_per_pair": raw_tot, "gzip_bytes_per_parent_sum": gz_tot,
    "gzip_bytes_per_parent": {"median": float(np.median(gz_per)), "p99": float(np.percentile(gz_per, 99)), "max": int(gz_per.max())},
    "children_per_parent": {"median": float(np.median(k_hist)), "p99": float(np.percentile(k_hist, 99)), "max": int(max(k_hist))},
}

top_kids = sorted(kids_of[0], key=lambda c: (-nodes[c], c))
res["top"] = parent_stats(0, with_order=True)[0]
big = [c for c in top_kids if len(kids_of.get(c, [])) >= 4][:a.preview_parents]
res["regions"] = [parent_stats(c, with_order=True)[0] for c in big]
# all region parents with at least 4 children: how much the strongest-links layer can show
cov = {str(b): [] for b in BANDS}
hits = []
for p in region_parents:
    if len(kids_of.get(p, [])) < 4:
        continue
    st = parent_stats(p)[0]
    for b in BANDS:
        cov[str(b)].append(st["bands_by_weight_share"][str(b)])
    if "top10_enriched_among_12_heaviest" in st and st["pairs_nonzero"] >= 22:
        hits.append(st["top10_enriched_among_12_heaviest"])
res["all_parents_k_ge_4"] = {"parents": len(cov["12"]),
                             "bands_share_median": {b: float(np.median(v)) for b, v in cov.items()},
                             "bands_share_p10": {b: float(np.percentile(v, 10)) for b, v in cov.items()},
                             "top10_enriched_among_12_heaviest_median": float(np.median(hits)) if hits else None,
                             "parents_with_22_or_more_pairs": len(hits)}

if a.hier:
    h = json.load(open(a.hier + "/hier.json"))
    L = len(h["levels"]) - 1
    qo = np.fromfile(f"{a.hier}/quot_{L}.offsets.u64", dtype=np.uint64).astype(np.int64)
    qt = np.fromfile(f"{a.hier}/quot_{L}.targets.u32", dtype=np.uint32).astype(np.int64)
    qw = np.fromfile(f"{a.hier}/quot_{L}.weights.f64", dtype=np.float64)
    # top regions that are each one community of the top hierarchy level: their pair weights and
    # internal edges must equal the quotient graph b2-leiden wrote (computed independently of this
    # script; a self-loop there weighs 2 x internal edges)
    kids, I, J, W = matrix(0)
    lv = tn["level"][kids] == L
    cm = tn["comm"][kids].astype(np.int64)
    theirs, loop = {}, {}
    for u in range(len(qo) - 1):
        for t_, w_ in zip(qt[qo[u]:qo[u + 1]].tolist(), qw[qo[u]:qo[u + 1]].tolist()):
            if u < t_:
                theirs[(u, t_)] = w_
            elif u == t_:
                loop[u] = w_
    sel = lv[I] & lv[J]
    mine = {(min(cm[i], cm[j]), max(cm[i], cm[j])): int(w) for i, j, w in zip(I[sel], J[sel], W[sel])}
    comms = set(cm[lv].tolist())
    theirs = {k_: v for k_, v in theirs.items() if k_[0] in comms and k_[1] in comms}
    both = set(mine) | set(theirs)
    res["hierarchy_check"] = {"top_level": L, "top_regions": len(kids), "top_regions_that_are_level_communities": int(lv.sum()),
                              "pairs_compared": len(both), "pairs_differing": sum(1 for k_ in both if mine.get(k_, 0) != theirs.get(k_, 0)),
                              "internal_differing": sum(1 for c, q in zip(np.array(kids)[lv], cm[lv]) if 2 * int(tn["internal"][c]) != loop.get(int(q), 0))}

json.dump(res, open(a.json, "w"), indent=1)
print(json.dumps({k: res[k] for k in ("graph", "oracle", "matrix_section")}, indent=1))

if a.preview:
    def R(v, d=5):
        return round(float(v), d)

    def parent_view(p):
        """One parent as the preview draws it: its children (with their own children as outlines),
        the per-child glyph figures, and the matrix with children under 0.1 % of the parent's
        nodes summed into one "+N smaller communities" row (sums stay exact). Rows are in the
        "spectral_enrichment" order measured above (Fiedler vector of the normalized Laplacian
        of the coupling the matrix shades)."""
        kids, I, J, W = matrix(p)
        k = len(kids)
        sib = np.bincount(I, weights=W, minlength=k) + np.bincount(J, weights=W, minlength=k)
        best = np.full(k, -1)
        bestw = np.zeros(k)
        for x, y, w in zip(np.r_[I, J], np.r_[J, I], np.r_[W, W]):
            if w > bestw[x] or (w == bestw[x] and best[x] > y):
                best[x], bestw[x] = y, w
        big_ = [i for i in range(k) if nodes[kids[i]] >= 0.001 * nodes[p]][:63]
        row = np.full(k, len(big_))
        row[big_] = np.arange(len(big_))
        nrow = len(big_) + (1 if len(big_) < k else 0)
        RI, RJ = row[I], row[J]
        same = RI == RJ
        key = np.minimum(RI, RJ)[~same] * nrow + np.maximum(RI, RJ)[~same]
        uk, inv = np.unique(key, return_inverse=True)
        RW = np.bincount(inv, weights=W[~same]).astype(np.int64)
        ri, rj = uk // nrow, uk % nrow
        rint = np.bincount(row, weights=tn["internal"][kids].astype(np.float64), minlength=nrow) + np.bincount(RI[same], weights=W[same], minlength=nrow)
        rvol = np.bincount(row, weights=vol[kids], minlength=nrow)
        rn = np.bincount(row, weights=nodes[kids].astype(np.float64), minlength=nrow)
        if nrow >= 4:
            A = np.zeros((nrow, nrow))
            A[ri, rj] = RW * (2.0 * m_und) / (rvol[ri] * rvol[rj])
            A = A + A.T
            d = A.sum(1)
            d[d == 0] = 1.0
            val, vec = np.linalg.eigh(np.eye(nrow) - A / np.sqrt(np.outer(d, d)))
            f = vec[:, 1] / np.sqrt(d)
            if f[np.argmax(np.abs(f))] < 0:
                f = -f
            perm = np.lexsort((np.arange(nrow), f)).tolist()
        else:
            perm = list(range(nrow))
        out = {"id": int(p), "x": R(tn["x"][p]), "y": R(tn["y"][p]), "r": R(tn["r"][p]), "n": int(nodes[p]), "int": int(tn["internal"][p]), "vol": int(vol[p]),
               "kids": [], "rows": [{"n": int(rn[i]), "int": int(rint[i]), "vol": int(rvol[i]), "agg": int((row == i).sum()) if i == len(big_) else 0,
                                     "kid": int(big_[i]) if i < len(big_) else -1} for i in range(nrow)],
               "pairs": [[int(x), int(y), int(w)] for x, y, w in zip(ri, rj, RW)], "order": perm}
        for i, c in enumerate(kids):
            gk = sorted([g for g in kids_of.get(c, []) if not tn["flags"][g] & F_AGG_INNER and tn["r"][g] >= 0.02 * tn["r"][c]], key=lambda g: (-nodes[g], g))
            out["kids"].append({"id": int(c), "x": R(tn["x"][c]), "y": R(tn["y"][c]), "r": R(tn["r"][c]), "n": int(nodes[c]), "int": int(tn["internal"][c]),
                                "vol": int(vol[c]), "f": int(tn["flags"][c]), "nk": len(kids_of.get(c, [])), "row": int(row[i]), "sib": int(sib[i]),
                                "tp": int(best[i]), "tpw": int(bestw[i]),
                                "c": [[R(tn["x"][g]), R(tn["y"][g]), R(tn["r"][g]), int(tn["flags"][g])] for g in gk]})
        return out

    pv = {"name": a.name, "n": n, "m": m_und, "parents": {"0": parent_view(0)}, "drill": [int(c) for c in big]}
    for c in big:
        pv["parents"][str(c)] = parent_view(c)
    body = json.dumps(pv, separators=(",", ":"))
    with open(a.preview, "w") as f:
        if a.preview.endswith(".js"):  # a script, so that the preview opens from file:// without a server
            f.write("// Generated by lab/m1-06-edge-rendering/overview.py from the lab map of %s. Exact counts.\n" % a.name)
            f.write("(window.OVERVIEW_DATA = window.OVERVIEW_DATA || {})[%s] = %s;\n" % (json.dumps(a.name), body))
        else:
            f.write(body)
