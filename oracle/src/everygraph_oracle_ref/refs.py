# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Reference runners: each kernel computed by NetworkX, python-igraph and scipy.

Every function returns the oracle's canonical output form: {"ints": [...]},
{"reals": [...]}, {"labels": [...]} (smallest node id of the class), {"count": n} or
{"real": x}; None marks an unreachable node. Where a library's convention differs
from Everygraph's on a case it can meet (oracle/SEMANTICS.md), the adjustment is made
here, explicitly, and listed in ADJUSTMENTS.
"""

from __future__ import annotations

import math

import networkx as nx
import numpy as np
import scipy.sparse.csgraph as csg

from .compare import canonical_labels
from .graph import Projection, to_igraph, to_networkx, to_scipy

# Library conventions mapped to Everygraph's, each with the SEMANTICS.md row it follows.
ADJUSTMENTS = {
    "igraph.transitivity_local_undirected": "mode='zero': degree < 2 gives 0, not NaN (row 'clustering, degree < 2')",
    "igraph.transitivity_undirected": "NaN (no connected triple) mapped to 0 (row 'transitivity, no triple')",
    "igraph.modularity": "NaN (no edges) mapped to 0 (row 'modularity, no edges')",
    "networkx.pagerank": "tol = 1e-14 / n, so that NetworkX's stopping rule (L1 change < n * tol) means L1 change < 1e-14 whatever n; retried with 1e-12 / n and 1e-10 / n if rounding noise prevents convergence; max_iter = 20000",
}

LIBS = ("networkx", "igraph", "scipy")


def _labels_from_sets(n: int, comps) -> list[int]:
    lab = [0] * n
    for c in comps:
        m = min(c)
        for v in c:
            lab[v] = m
    return lab


def _labels_from_membership(memb) -> list[int]:
    return canonical_labels(list(memb))


def _dist_ints(row) -> list:
    return [None if math.isinf(x) else int(x) for x in row]


def _dist_reals(row) -> list:
    return [None if math.isinf(x) else float(x) for x in row]


def _nx_pagerank(G, damping, weighted):
    """NetworkX PageRank as close to the fixed point as its stopping rule allows.

    NetworkX stops when the L1 change between iterates is below n * tol, so tol is scaled
    by 1/n. Where rounding noise keeps the L1 change above the tightest bound (seen on a
    star with 2^16 leaves), looser bounds are tried; the comparison with igraph and the
    in-process reference then decides whether the answer is close enough.
    """
    n = G.number_of_nodes()
    if n == 0:
        return []
    weight = "wsum" if weighted else None
    for bound in (1e-14, 1e-12, 1e-10):
        try:
            pr = nx.pagerank(G, alpha=damping, max_iter=20000, tol=bound / n, weight=weight)
            return [pr[v] for v in range(n)]
        except nx.PowerIterationFailedConvergence:
            continue
    raise nx.PowerIterationFailedConvergence(20000)


def _triangles_scipy(p: Projection) -> list[int]:
    """Triangles per node with sparse products over the degree-ordered orientation.

    Not `diag(A^3) / 2`: `A @ A` has deg(v)^2 entries at a hub, which is 2^32 for the
    nightly corpus's star with 2^16 leaves (it once took 61 GB). Orienting each edge from
    the endpoint of lower (degree, id) to the higher one bounds every product below by
    the sum of squared forward degrees, which is O(m^1.5).
    """
    import scipy.sparse as sp

    if p.n == 0:
        return []
    A = to_scipy(p, undirected=True).tocoo()
    deg = np.bincount(A.row, minlength=p.n)
    key = deg.astype(np.int64) * (p.n + 1) + np.arange(p.n)
    fwd = key[A.row] < key[A.col]
    U = sp.csr_array((np.ones(int(fwd.sum())), (A.row[fwd], A.col[fwd])), shape=(p.n, p.n))
    # For a triangle u < w < x in that order: (U @ U)[u, x] counts w.
    C = (U @ U).multiply(U)
    first = np.asarray(C.sum(axis=1)).ravel()
    last = np.asarray(C.sum(axis=0)).ravel()
    middle = np.asarray((U.multiply(U.T @ U)).sum(axis=1)).ravel()
    return [int(round(x)) for x in first + last + middle]


def compute(p: Projection, partition, kernel: dict) -> dict[str, dict]:
    """The answer of each library that implements `kernel`, keyed by library name."""
    k = kernel["kernel"]
    n = p.n
    out: dict[str, dict] = {}

    if k in ("degree", "degree_out", "degree_in"):
        loops = p.keep_loops
        G = to_networkx(p, loops=loops)
        H = to_igraph(p, loops=loops)
        if k == "degree":
            out["networkx"] = {"ints": [G.degree(v) for v in range(n)]}
            out["igraph"] = {"ints": H.degree(mode="all", loops=True)}
        elif k == "degree_out":
            out["networkx"] = {"ints": [G.out_degree(v) for v in range(n)]}
            out["igraph"] = {"ints": H.degree(mode="out", loops=True)}
        else:
            out["networkx"] = {"ints": [G.in_degree(v) for v in range(n)]}
            out["igraph"] = {"ints": H.degree(mode="in", loops=True)}
        A = to_scipy(p)
        rows = np.diff(A.indptr).astype(int).tolist() if n else []
        cols = np.bincount(A.indices, minlength=n).astype(int).tolist() if n else []
        loop_set = set(p.loop_nodes) if loops else set()
        loop = [1 if v in loop_set else 0 for v in range(n)]
        if k == "degree":
            base = [r + c for r, c in zip(rows, cols)] if p.directed else rows
            out["scipy"] = {"ints": [b + 2 * lp for b, lp in zip(base, loop)]}
        elif k == "degree_out":
            out["scipy"] = {"ints": [r + lp for r, lp in zip(rows, loop)]}
        else:
            out["scipy"] = {"ints": [c + lp for c, lp in zip(cols, loop)]}
        return out

    if k == "pagerank":
        d, weighted = kernel["damping"], kernel["weighted"]
        out["networkx"] = {"reals": _nx_pagerank(to_networkx(p), d, weighted)}
        H = to_igraph(p)
        if n:
            pr = H.pagerank(damping=d, directed=p.directed, weights="wsum" if weighted else None, implementation="prpack")
        else:
            pr = []
        out["igraph"] = {"reals": list(pr)}
        return out

    if k in ("wcc", "scc"):
        strong = k == "scc"
        G = to_networkx(p)
        if strong:
            comps = nx.strongly_connected_components(G)
        elif p.directed:
            comps = nx.weakly_connected_components(G)
        else:
            comps = nx.connected_components(G)
        out["networkx"] = {"labels": _labels_from_sets(n, comps)}
        H = to_igraph(p)
        out["igraph"] = {"labels": _labels_from_membership(H.connected_components(mode="strong" if strong else "weak").membership)}
        if n:
            _, lab = csg.connected_components(to_scipy(p), directed=p.directed, connection="strong" if strong else "weak")
            out["scipy"] = {"labels": _labels_from_membership(lab.tolist())}
        else:
            out["scipy"] = {"labels": []}
        return out

    if k == "bfs":
        s = kernel["source"]
        G = to_networkx(p)
        dist = nx.single_source_shortest_path_length(G, s)
        out["networkx"] = {"ints": [dist.get(v) for v in range(n)]}
        out["igraph"] = {"ints": _dist_ints(to_igraph(p).distances(source=[s], mode="out")[0])}
        sc = csg.shortest_path(to_scipy(p), directed=p.directed, unweighted=True, indices=s)
        out["scipy"] = {"ints": _dist_ints(sc.tolist())}
        return out

    if k == "sssp":
        s = kernel["source"]
        G = to_networkx(p)
        dist = nx.single_source_dijkstra_path_length(G, s, weight="wmin")
        out["networkx"] = {"reals": [float(dist[v]) if v in dist else None for v in range(n)]}
        out["igraph"] = {"reals": _dist_reals(to_igraph(p).distances(source=[s], weights="wmin", mode="out")[0])}
        sc = csg.dijkstra(to_scipy(p, weight="wmin"), directed=p.directed, indices=s)
        out["scipy"] = {"reals": _dist_reals(sc.tolist())}
        return out

    Gu = to_networkx(p, undirected=True)
    Hu = to_igraph(p, undirected=True)

    if k == "core_number":
        cn = nx.core_number(Gu)
        out["networkx"] = {"ints": [cn[v] for v in range(n)]}
        out["igraph"] = {"ints": Hu.coreness(mode="all")}
        return out

    if k in ("triangles", "triangle_total"):
        t = nx.triangles(Gu)
        per = [0] * n
        for tri in Hu.list_triangles():
            for v in tri:
                per[v] += 1
        sc = _triangles_scipy(p)
        if k == "triangles":
            out["networkx"] = {"ints": [t[v] for v in range(n)]}
            out["igraph"] = {"ints": per}
            out["scipy"] = {"ints": sc}
        else:
            out["networkx"] = {"count": sum(t.values()) // 3}
            out["igraph"] = {"count": len(Hu.list_triangles())}
            out["scipy"] = {"count": sum(sc) // 3}
        return out

    if k == "clustering":
        c = nx.clustering(Gu)
        out["networkx"] = {"reals": [float(c[v]) for v in range(n)]}
        out["igraph"] = {"reals": [float(x) for x in Hu.transitivity_local_undirected(mode="zero")]}
        return out

    if k == "transitivity":
        out["networkx"] = {"real": float(nx.transitivity(Gu))}
        t = Hu.transitivity_undirected()
        out["igraph"] = {"real": 0.0 if math.isnan(t) else float(t)}
        return out

    if k == "modularity":
        if partition is None:
            return out
        gamma = kernel["resolution"]
        groups: dict[int, set[int]] = {}
        for v, c in enumerate(partition):
            groups.setdefault(c, set()).add(v)
        out["networkx"] = {"real": float(nx.community.modularity(Gu, list(groups.values()), weight=None, resolution=gamma))}
        q = Hu.modularity(partition, weights=None, resolution=gamma, directed=False)
        out["igraph"] = {"real": 0.0 if math.isnan(q) else float(q)}
        return out

    return out
