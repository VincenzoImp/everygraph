#!/usr/bin/env python3
"""metrics.py --graph CSR --hier HIER --gt CMTY.gz --pos POS.f32 --name NAME --out OUT.json

One metric set for every layout of the same graph. POS.f32 holds n x 2 little-endian f32
(x, y) in the CSR's dense node order. All samples use fixed seeds, so every layout is scored
on the same nodes, pairs and sources.

  edge ratios      edge length / random-pair distance: median/median, mean/mean,
                   p90/median (the D17 framing) and p90/p90; edge-length CV (Hachul-Juenger)
  NP1, NP2         Jaccard of the graph 1- and 2-hop ball of a node with its |ball| nearest
                   nodes in the drawing (Kruiger et al.), on sampled nodes
  quotient NP      the same on each hierarchy level: community = centroid of its members'
                   positions; NP1 over all quotient neighbours and NP@10 over the 10
                   strongest (w_ij / sqrt(vol_i vol_j)) against the 10 nearest centroids
  stress           sum (a*|xu-xv| - d_uv)^2 / d_uv^2 at the optimal scale a, normalised by
                   the number of pairs, on BFS-sampled pairs (Smelser et al.: compare at a*)
  ground truth     SNAP top-5000 communities, nodes in exactly one of them: silhouette of
                   the labels in the drawing, 10-NN label agreement among labelled nodes,
                   and ARI of k-means (k = number of communities) of their positions
"""
import argparse, gzip, json, time
import numpy as np
from scipy.sparse import csr_matrix
from scipy.sparse.csgraph import shortest_path
from scipy.spatial import cKDTree
from scipy.stats import spearmanr
from sklearn.cluster import KMeans
from sklearn.metrics import adjusted_rand_score, silhouette_score

ap = argparse.ArgumentParser()
ap.add_argument("--graph", required=True)
ap.add_argument("--hier", required=True)
ap.add_argument("--gt", required=True)
ap.add_argument("--pos", required=True)
ap.add_argument("--name", required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--np-nodes", type=int, default=10000)
ap.add_argument("--np2-cap", type=int, default=5000)
ap.add_argument("--stress-sources", type=int, default=100)
ap.add_argument("--stress-targets", type=int, default=1000)
ap.add_argument("--skip", default="", help="comma list of: np,quot,stress,gt")
a = ap.parse_args()
skip = set(filter(None, a.skip.split(",")))
T0 = time.time()
res = {"layout": a.name, "pos": a.pos, "seeds": "fixed (numpy default_rng 1..5)"}


def log(*x):
    print(f"[{time.time() - T0:7.1f}s]", *x, flush=True)


meta = json.load(open(f"{a.graph}/meta.json"))
n = meta["n"]
off = np.fromfile(f"{a.graph}/offsets.u64", dtype=np.uint64).astype(np.int64)
tgt = np.fromfile(f"{a.graph}/targets.u32", dtype=np.uint32)
deg = np.diff(off)
pos = np.fromfile(a.pos, dtype=np.float32).reshape(-1, 2).astype(np.float64)
assert len(pos) == n, (len(pos), n)
assert np.isfinite(pos).all(), "non-finite positions"
res["n"] = n

# ---- edge ratios
src = np.repeat(np.arange(n, dtype=np.int64), deg)
keep = tgt > src
eu, ev = src[keep], tgt[keep].astype(np.int64)
del src, keep
el = np.hypot(*(pos[eu] - pos[ev]).T)
rng = np.random.default_rng(1)
ru, rv = rng.integers(0, n, 10_000_000), rng.integers(0, n, 10_000_000)
rl = np.hypot(*(pos[ru] - pos[rv]).T)
del ru, rv
r50, r90, rmean = np.median(rl), np.quantile(rl, 0.9), rl.mean()
e50, e90, emean = np.median(el), np.quantile(el, 0.9), el.mean()
res["edges"] = {
    "m": int(len(el)), "random_pairs": 10_000_000,
    "median_ratio": e50 / r50, "mean_ratio": emean / rmean,
    "p90_over_random_median": e90 / r50, "p90_ratio": e90 / r90,
    "edge_len_cv": el.std() / emean,
    "frac_shorter_than_1pct_of_random_median": float((el < 0.01 * r50).mean()),
}
del el, eu, ev, rl
log("edges", res["edges"])

tree = cKDTree(pos)


def adj(u):
    return tgt[off[u]:off[u + 1]]


def np_ball(nodes, balls):
    """mean Jaccard of each ball with the |ball| nearest other points of the node."""
    out = []
    for u, b in zip(nodes, balls):
        k = len(b)
        _, nn = tree.query(pos[u], k=k + 1)
        nn = nn[nn != u][:k]
        inter = np.intersect1d(nn, b, assume_unique=True).size
        out.append(inter / (2 * k - inter))
    return float(np.mean(out)) if out else None


# ---- node-level NP1 / NP2
if "np" not in skip:
    rng = np.random.default_rng(2)
    nodes = rng.choice(n, a.np_nodes, replace=False)
    res["np1"] = np_ball(nodes, [adj(u).astype(np.int64) for u in nodes])
    b2, kept = [], []
    for u in nodes:
        nb = adj(u)
        if deg[nb].sum() > 4 * a.np2_cap:
            continue
        ball = np.unique(np.concatenate([nb] + [adj(w) for w in nb]))
        ball = ball[ball != u].astype(np.int64)
        if len(ball) <= a.np2_cap:
            b2.append(ball)
            kept.append(u)
    res["np2"] = np_ball(kept, b2)
    res["np_nodes"] = {"np1": len(nodes), "np2": len(kept), "np2_cap": a.np2_cap}
    log("np", res["np1"], res["np2"], res["np_nodes"])

# ---- quotient-level NP
hier = json.load(open(f"{a.hier}/hier.json"))
comm = np.fromfile(f"{a.hier}/level_0.u32", dtype=np.uint32).astype(np.int64)
levels = []
if "quot" not in skip:
    for k in range(len(hier["levels"])):
        if k > 0:
            comm = np.fromfile(f"{a.hier}/level_{k}.u32", dtype=np.uint32).astype(np.int64)[comm]
        nc = int(comm.max()) + 1
        cnt = np.bincount(comm, minlength=nc)
        cen = np.stack([np.bincount(comm, pos[:, 0], nc), np.bincount(comm, pos[:, 1], nc)], 1) / cnt[:, None]
        vol = np.fromfile(f"{a.hier}/level_{k}.volume.u64", dtype=np.uint64).astype(np.float64)
        qo = np.fromfile(f"{a.hier}/quot_{k}.offsets.u64", dtype=np.uint64).astype(np.int64)
        qt = np.fromfile(f"{a.hier}/quot_{k}.targets.u32", dtype=np.uint32).astype(np.int64)
        qw = np.fromfile(f"{a.hier}/quot_{k}.weights.f64", dtype=np.float64)
        ct = cKDTree(cen)
        rng = np.random.default_rng(3)
        sample = rng.choice(nc, min(nc, 10000), replace=False)
        j1, j10 = [], []
        for c in sample:
            t, w = qt[qo[c]:qo[c + 1]], qw[qo[c]:qo[c + 1]]
            nz = t != c
            t, w = t[nz], w[nz]
            if len(t) == 0 or len(t) >= nc - 1:
                continue
            for kk, nb, acc in ((len(t), t, j1), (min(10, len(t)), t[np.argsort(-w / np.sqrt(vol[c] * vol[t]), kind="stable")[:10]], j10)):
                _, nn = ct.query(cen[c], k=kk + 1)
                nn = np.atleast_1d(nn)
                nn = nn[nn != c][:kk]
                inter = np.intersect1d(nn, nb).size
                acc.append(inter / (2 * kk - inter))
        levels.append({"level": k, "communities": nc, "sampled": len(j1),
                       "np1": float(np.mean(j1)) if j1 else None,
                       "np_top10": float(np.mean(j10)) if j10 else None,
                       "mean_quotient_degree": float((np.diff(qo) - 1).mean())})
        log("quot", levels[-1])
    res["quotient_np"] = levels

# ---- stress at optimal scale
if "stress" not in skip:
    g = csr_matrix((np.ones(len(tgt), dtype=np.float32), tgt, off), shape=(n, n))
    rng = np.random.default_rng(4)
    sources = rng.choice(n, a.stress_sources, replace=False)
    D, E = [], []
    for i in range(0, len(sources), 10):
        b = sources[i:i + 10]
        d = shortest_path(g, unweighted=True, directed=False, indices=b)
        for row, s in zip(d, b):
            t = rng.integers(0, n, a.stress_targets)
            dd = row[t]
            ok = np.isfinite(dd) & (dd > 0)
            D.append(dd[ok])
            E.append(np.hypot(*(pos[t[ok]] - pos[s]).T))
    D, E = np.concatenate(D), np.concatenate(E)
    w = 1.0 / D**2
    alpha = (w * D * E).sum() / (w * E * E).sum()
    s = (w * (alpha * E - D) ** 2).sum() / len(D)
    res["stress"] = {"pairs": int(len(D)), "sources": a.stress_sources, "alpha": float(alpha),
                     "normalized_stress_at_optimal_scale": float(s),
                     "spearman_layout_vs_graph_distance": float(spearmanr(D, E).statistic),
                     "mean_hops": float(D.mean())}
    log("stress", res["stress"])

# ---- SNAP ground-truth communities
if "gt" not in skip:
    ids = np.fromfile(f"{a.graph}/ids.u64", dtype=np.uint64)
    order = np.argsort(ids)
    lab = np.full(n, -1, dtype=np.int64)
    seen = np.zeros(n, dtype=np.int32)
    with gzip.open(a.gt, "rt") as f:
        for ci, line in enumerate(f):
            orig = np.array(line.split(), dtype=np.uint64)
            j = np.searchsorted(ids, orig, sorter=order)
            dense = order[np.minimum(j, n - 1)]
            dense = dense[ids[dense] == orig]
            seen[dense] += 1
            lab[dense] = ci
    lab[seen != 1] = -1
    m = lab >= 0
    L = lab[m]
    uniq, L = np.unique(L, return_inverse=True)
    cnt = np.bincount(L)
    keep = cnt[L] >= 2
    idx = np.flatnonzero(m)[keep]
    L = np.unique(L[keep], return_inverse=True)[1]
    P = pos[idx]
    k = int(L.max()) + 1
    sil = silhouette_score(P, L) if len(idx) <= 120_000 else silhouette_score(P, L, sample_size=120_000, random_state=5)
    _, nn = cKDTree(P).query(P, k=11)
    knn_agree = float((L[nn[:, 1:]] == L[:, None]).mean())
    km = KMeans(n_clusters=k, n_init=1, random_state=5).fit_predict(P)
    # Reference, independent of the drawing: how well each hierarchy level itself matches.
    hc, href = np.fromfile(f"{a.hier}/level_0.u32", dtype=np.uint32).astype(np.int64), []
    for lv in range(len(hier["levels"])):
        if lv > 0:
            hc = np.fromfile(f"{a.hier}/level_{lv}.u32", dtype=np.uint32).astype(np.int64)[hc]
        href.append(float(adjusted_rand_score(L, hc[idx])))
    res["ground_truth"] = {"source": a.gt, "labelled_nodes": int(len(idx)), "communities": k,
                           "silhouette": float(sil), "knn10_label_agreement": knn_agree,
                           "kmeans_ari": float(adjusted_rand_score(L, km)),
                           "chance_knn10": float(((np.bincount(L) / len(L)) ** 2).sum()),
                           "hierarchy_level_ari_reference": href}
    log("gt", res["ground_truth"])

res["metrics_s"] = round(time.time() - T0, 1)
json.dump(res, open(a.out, "w"), indent=1)
log("done", a.out)
