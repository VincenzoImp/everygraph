# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Semantics probe: what NetworkX and python-igraph do on the graph kinds where they may
disagree (self-loops, parallel edges, direction, degenerate inputs), next to the answer
the oracle expects from Everygraph. Writes oracle/semantics/probe.json, the evidence
behind oracle/SEMANTICS.md; `--check` fails if a library upgrade changed any answer.

Graphs are given to each library *raw* (MultiGraph / MultiDiGraph in NetworkX, a
multigraph in igraph), so that the libraries' own conventions show. The Everygraph
column is computed on the analytic graph (refs.compute on the projection), the same
answer the Rust references give.
"""

from __future__ import annotations

import json
import math
from pathlib import Path

import igraph as ig
import networkx as nx

from .graph import Graph, project
from .refs import compute

ORACLE = Path(__file__).resolve().parents[2]
OUT = ORACLE / "semantics" / "probe.json"


def _round(x):
    if isinstance(x, float):
        if math.isnan(x):
            return "NaN"
        if math.isinf(x):
            return None  # unreachable: NetworkX omits the node, igraph says inf
        return round(x, 6)
    if isinstance(x, dict):
        return {k: _round(v) for k, v in x.items()}
    if isinstance(x, (list, tuple)):
        return [_round(v) for v in x]
    return x


def _try(f):
    try:
        return _round(f())
    except Exception as e:  # the refusal itself is the answer
        return f"refuses: {type(e).__name__}: {str(e).splitlines()[0][:70]}"


def _raw(n, edges, directed, weights=None):
    G = nx.MultiDiGraph() if directed else nx.MultiGraph()
    G.add_nodes_from(range(n))
    for i, (u, v) in enumerate(edges):
        G.add_edge(u, v, weight=1.0 if weights is None else weights[i])
    H = ig.Graph(n=n, edges=edges, directed=directed)
    H.es["weight"] = [1.0] * len(edges) if weights is None else weights
    return G, H


def _ours(n, edges, directed, kernel, partition=None, weights=None, keep_loops=False):
    g = Graph(
        name="probe",
        n=n,
        directed=directed,
        src=[u for u, _ in edges],
        dst=[v for _, v in edges],
        weight=weights,
        time=None,
        partition=partition,
        keep_loops=keep_loops,
        kernels=[kernel],
    )
    vals = compute(project(g), partition, kernel)
    out = vals.get("networkx") or next(iter(vals.values()))
    (_, v), = out.items()
    return _round(v)


def _nodes(d):
    return [d[k] for k in sorted(d)]


TRI = [(0, 1), (1, 2), (2, 0)]
LOOP = [(0, 0)] + TRI
MULTI = [(0, 1), (0, 1), (1, 2), (2, 0), (2, 3)]
DIR = [(0, 1), (1, 2), (2, 0), (0, 2), (3, 0)]


def cases() -> list[dict]:
    rows = []

    def row(id_, question, graph, nx_f, ig_f, ours):
        rows.append({"id": id_, "question": question, "graph": graph, "networkx": _try(nx_f), "igraph": _try(ig_f), "everygraph": ours})

    G, H = _raw(3, LOOP, False)
    row("loop-degree", "degree of a node with a self-loop", "triangle + loop at 0, undirected",
        lambda: _nodes(dict(G.degree())), lambda: H.degree(),
        _ours(3, LOOP, False, {"kernel": "degree"}, keep_loops=True))
    row("loop-degree-dropped", "degree with the default semantics (loops dropped)", "triangle + loop at 0, undirected",
        lambda: "n/a (no drop option)", lambda: "n/a (no drop option)",
        _ours(3, LOOP, False, {"kernel": "degree"}))
    # From here on the Everygraph column keeps the loops (docs/spec/graph-semantics.md S1-S6).
    row("loop-core", "core numbers with a kept self-loop", "triangle + loop at 0, undirected",
        lambda: _nodes(nx.core_number(nx.Graph(G))), lambda: H.coreness(),
        _ours(3, LOOP, False, {"kernel": "core_number"}, keep_loops=True))
    G1, H1 = _raw(2, [(0, 0)], False)
    row("loop-core-lone", "core numbers of a lone node with a kept self-loop", "loop at 0, node 1 isolated",
        lambda: _nodes(nx.core_number(nx.Graph(G1))), lambda: H1.coreness(),
        _ours(2, [(0, 0)], False, {"kernel": "core_number"}, keep_loops=True))
    E2 = [(0, 0), (1, 1), (0, 1)]
    G2l, H2l = _raw(2, E2, False)
    row("loop-core-pair", "core numbers of two linked nodes with a kept self-loop each", "edge 0-1, loops at 0 and 1",
        lambda: _nodes(nx.core_number(nx.Graph(G2l))), lambda: H2l.coreness(),
        _ours(2, E2, False, {"kernel": "core_number"}, keep_loops=True))
    row("loop-pagerank", "PageRank with a kept self-loop", "triangle + loop at 0, undirected",
        lambda: _nodes(nx.pagerank(nx.Graph(G), tol=1e-15, max_iter=10000)), lambda: H.pagerank(),
        _ours(3, LOOP, False, {"kernel": "pagerank", "damping": 0.85, "weighted": False}, keep_loops=True))
    W3 = [3.0, 1.0, 1.0, 1.0]
    Gw3, Hw3 = _raw(3, LOOP, False, W3)
    row("loop-pagerank-weighted", "weighted PageRank with a kept self-loop of weight 3", "triangle + loop at 0 of weight 3, undirected",
        lambda: _nodes(nx.pagerank(nx.Graph(Gw3), tol=1e-15, max_iter=10000)), lambda: Hw3.pagerank(weights="weight"),
        _ours(3, LOOP, False, {"kernel": "pagerank", "damping": 0.85, "weighted": True}, weights=W3, keep_loops=True))
    Gd, Hd = _raw(3, [(0, 0), (0, 1), (1, 2), (2, 0)], True)
    row("loop-pagerank-directed", "PageRank with a kept self-loop, directed", "3-cycle + loop at 0, directed",
        lambda: _nodes(nx.pagerank(nx.DiGraph(Gd), tol=1e-15, max_iter=10000)), lambda: Hd.pagerank(),
        _ours(3, [(0, 0), (0, 1), (1, 2), (2, 0)], True, {"kernel": "pagerank", "damping": 0.85, "weighted": False}, keep_loops=True))
    row("loop-triangles", "triangles with a kept self-loop", "triangle + loop at 0, undirected",
        lambda: _nodes(nx.triangles(nx.Graph(G))), lambda: [sum(v in t for t in H.list_triangles()) for v in range(3)],
        _ours(3, LOOP, False, {"kernel": "triangles"}, keep_loops=True))
    row("loop-clustering", "clustering with a kept self-loop", "triangle + loop at 0, undirected",
        lambda: _nodes(nx.clustering(nx.Graph(G))), lambda: H.transitivity_local_undirected(mode="zero"),
        _ours(3, LOOP, False, {"kernel": "clustering"}, keep_loops=True))
    row("loop-transitivity", "transitivity with a kept self-loop", "triangle + loop at 0, undirected",
        lambda: nx.transitivity(nx.Graph(G)), lambda: H.transitivity_undirected(),
        _ours(3, LOOP, False, {"kernel": "transitivity"}, keep_loops=True))
    LP = LOOP + [(2, 3)]
    Gp4, Hp4 = _raw(4, LP, False)
    row("loop-clustering-pendant", "clustering with a kept self-loop and a pendant", "triangle + loop at 0 + pendant 2-3, undirected",
        lambda: _nodes(nx.clustering(nx.Graph(Gp4))), lambda: Hp4.transitivity_local_undirected(mode="zero"),
        _ours(4, LP, False, {"kernel": "clustering"}, keep_loops=True))
    row("loop-transitivity-pendant", "transitivity with a kept self-loop and a pendant", "same",
        lambda: nx.transitivity(nx.Graph(Gp4)), lambda: Hp4.transitivity_undirected(),
        _ours(4, LP, False, {"kernel": "transitivity"}, keep_loops=True))
    row("loop-modularity", "modularity with a kept self-loop, partition {0,1} {2}", "triangle + loop at 0, undirected",
        lambda: nx.community.modularity(nx.Graph(G), [{0, 1}, {2}]), lambda: H.modularity([0, 0, 1]),
        _ours(3, LOOP, False, {"kernel": "modularity", "resolution": 1.0}, partition=[0, 0, 1], keep_loops=True))
    row("loop-modularity-weighted", "weighted modularity with a kept self-loop of weight 3, partition {0,1} {2}", "triangle + loop at 0 of weight 3, undirected",
        lambda: nx.community.modularity(nx.Graph(Gw3), [{0, 1}, {2}], weight="weight"), lambda: Hw3.modularity([0, 0, 1], weights="weight"),
        _ours(3, LOOP, False, {"kernel": "modularity", "resolution": 1.0, "weighted": True}, partition=[0, 0, 1], weights=W3, keep_loops=True))
    row("loop-modularity-directed", "modularity of a directed graph with a kept self-loop, partition {0,1} {2}", "3-cycle + loop at 0, directed",
        lambda: nx.community.modularity(nx.DiGraph(Gd), [{0, 1}, {2}]), lambda: Hd.modularity([0, 0, 1]),
        _ours(3, [(0, 0), (0, 1), (1, 2), (2, 0)], True, {"kernel": "modularity", "resolution": 1.0}, partition=[0, 0, 1], keep_loops=True))

    G, H = _raw(4, MULTI, False)
    row("multi-degree", "degree with a repeated pair", "triangle + pendant 3, pair 0-1 twice, undirected",
        lambda: _nodes(dict(G.degree())), lambda: H.degree(),
        _ours(4, MULTI, False, {"kernel": "degree"}))
    row("multi-core", "core numbers with a repeated pair", "same",
        lambda: _nodes(nx.core_number(G)), lambda: H.coreness(),
        _ours(4, MULTI, False, {"kernel": "core_number"}))
    row("multi-triangles", "triangles with a repeated pair", "same",
        lambda: _nodes(nx.triangles(G)), lambda: [sum(v in t for t in H.list_triangles()) for v in range(4)],
        _ours(4, MULTI, False, {"kernel": "triangles"}))
    row("multi-clustering", "clustering with a repeated pair", "same",
        lambda: _nodes(nx.clustering(G)), lambda: H.transitivity_local_undirected(mode="zero"),
        _ours(4, MULTI, False, {"kernel": "clustering"}))
    row("multi-transitivity", "transitivity with a repeated pair", "same",
        lambda: nx.transitivity(G), lambda: H.transitivity_undirected(),
        _ours(4, MULTI, False, {"kernel": "transitivity"}))
    row("multi-pagerank", "PageRank with a repeated pair", "same",
        lambda: _nodes(nx.pagerank(G, tol=1e-15, max_iter=10000)), lambda: H.pagerank(),
        _ours(4, MULTI, False, {"kernel": "pagerank", "damping": 0.85, "weighted": False}))
    row("multi-pagerank-weighted", "PageRank weighted by multiplicity", "same",
        lambda: _nodes(nx.pagerank(G, tol=1e-15, max_iter=10000)), lambda: H.pagerank(weights="weight"),
        _ours(4, MULTI, False, {"kernel": "pagerank", "damping": 0.85, "weighted": True}))
    row("multi-modularity", "modularity with a repeated pair, partition {0,1} {2,3}", "same",
        lambda: nx.community.modularity(G, [{0, 1}, {2, 3}]), lambda: H.modularity([0, 0, 1, 1]),
        _ours(4, MULTI, False, {"kernel": "modularity", "resolution": 1.0}, partition=[0, 0, 1, 1]))
    w = [5.0, 1.0, 1.0, 1.0, 1.0]
    Gw, Hw = _raw(4, MULTI, False, w)
    row("multi-sssp", "weighted distance from 0 with parallel weights 5 and 1 on 0-1", "same, weights 5,1,1,1,1",
        lambda: _nodes(nx.single_source_dijkstra_path_length(Gw, 0)), lambda: Hw.distances(source=[0], weights="weight")[0],
        _ours(4, MULTI, False, {"kernel": "sssp", "source": 0}, weights=w))

    G, H = _raw(4, DIR, True)
    row("dir-degree", "degree of a directed graph", "3-cycle 0-1-2 + arc 0->2 + arc 3->0",
        lambda: _nodes(dict(G.degree())), lambda: H.degree(),
        _ours(4, DIR, True, {"kernel": "degree"}))
    row("dir-core", "core numbers of a directed graph", "same",
        lambda: _nodes(nx.core_number(nx.DiGraph(G))), lambda: H.coreness(),
        _ours(4, DIR, True, {"kernel": "core_number"}))
    row("dir-core-reciprocal", "core numbers of a directed graph with reciprocal arcs", "0->1 and 1->0 only",
        lambda: _nodes(nx.core_number(nx.DiGraph([(0, 1), (1, 0)]))), lambda: ig.Graph(n=2, edges=[(0, 1), (1, 0)], directed=True).coreness(),
        _ours(2, [(0, 1), (1, 0)], True, {"kernel": "core_number"}))
    row("dir-triangles", "triangles of a directed graph", "same",
        lambda: _nodes(nx.triangles(nx.DiGraph(G))), lambda: [sum(v in t for t in H.list_triangles()) for v in range(4)],
        _ours(4, DIR, True, {"kernel": "triangles"}))
    row("dir-clustering", "clustering of a directed graph", "same",
        lambda: _nodes(nx.clustering(nx.DiGraph(G))), lambda: H.transitivity_local_undirected(mode="zero"),
        _ours(4, DIR, True, {"kernel": "clustering"}))
    row("dir-transitivity", "transitivity of a directed graph", "same",
        lambda: nx.transitivity(nx.DiGraph(G)), lambda: H.transitivity_undirected(),
        _ours(4, DIR, True, {"kernel": "transitivity"}))
    D2 = [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (0, 3)]
    G2, H2 = _raw(6, D2, True)
    row("dir-modularity", "modularity of a directed graph, partition {0,1,2} {3,4,5}", "two directed 3-cycles joined by 0->3",
        lambda: nx.community.modularity(nx.DiGraph(G2), [{0, 1, 2}, {3, 4, 5}]), lambda: H2.modularity([0, 0, 0, 1, 1, 1]),
        _ours(6, D2, True, {"kernel": "modularity", "resolution": 1.0}, partition=[0, 0, 0, 1, 1, 1]))
    G3, H3 = _raw(3, [(0, 1), (1, 2)], True)
    row("dir-bfs", "BFS distances from 2 on 0->1->2", "directed path",
        lambda: [nx.single_source_shortest_path_length(nx.DiGraph(G3), 2).get(v) for v in range(3)],
        lambda: H3.distances(source=[2])[0],
        _ours(3, [(0, 1), (1, 2)], True, {"kernel": "bfs", "source": 2}))
    G4, H4 = _raw(4, [(0, 1), (1, 2)], True)
    row("dir-pagerank-dangling", "PageRank with dangling nodes", "0->1->2, node 3 isolated",
        lambda: _nodes(nx.pagerank(nx.DiGraph(G4), tol=1e-15, max_iter=10000)), lambda: H4.pagerank(),
        _ours(4, [(0, 1), (1, 2)], True, {"kernel": "pagerank", "damping": 0.85, "weighted": False}))

    G5, H5 = _raw(3, [(0, 1)], False)
    row("lowdeg-clustering", "clustering where degree < 2", "one edge + isolate",
        lambda: _nodes(nx.clustering(nx.Graph(G5))), lambda: H5.transitivity_local_undirected(),
        _ours(3, [(0, 1)], False, {"kernel": "clustering"}))
    row("notriple-transitivity", "transitivity without any connected triple", "one edge + isolate",
        lambda: nx.transitivity(nx.Graph(G5)), lambda: H5.transitivity_undirected(),
        _ours(3, [(0, 1)], False, {"kernel": "transitivity"}))
    G6, H6 = _raw(2, [], False)
    row("noedge-modularity", "modularity of a graph without edges", "two isolated nodes, partition {0} {1}",
        lambda: nx.community.modularity(nx.Graph(G6), [{0}, {1}]), lambda: H6.modularity([0, 1]),
        _ours(2, [], False, {"kernel": "modularity", "resolution": 1.0}, partition=[0, 1]))
    G7, H7 = _raw(0, [], False)
    row("empty-pagerank", "PageRank of the empty graph", "no nodes",
        lambda: _nodes(nx.pagerank(nx.Graph(G7))), lambda: H7.pagerank(),
        _ours(0, [], False, {"kernel": "pagerank", "damping": 0.85, "weighted": False}))

    # PageRank convergence: NetworkX stops when the L1 change < n * tol (default 1e-6).
    import random

    rnd = random.Random(7)
    n = 3000
    E = [(rnd.randrange(n), rnd.randrange(n)) for _ in range(9000)]
    E = [(u, v) for u, v in E if u != v]
    Gp = nx.DiGraph()
    Gp.add_nodes_from(range(n))
    Gp.add_edges_from(E)
    Hp = ig.Graph(n=n, edges=list(Gp.edges()), directed=True)
    ref = Hp.pagerank()
    default = nx.pagerank(Gp)
    tight = nx.pagerank(Gp, tol=1e-15, max_iter=100000)
    rows.append({
        "id": "pagerank-convergence",
        "question": "how far NetworkX's default stopping rule (L1 change < n * 1e-6) is from the fixed point",
        "graph": "random directed graph, n = 3000, about 9000 arcs, seed 7",
        "networkx": {"default_tol_max_abs_error_vs_prpack": _round(max(abs(default[v] - ref[v]) for v in range(n))),
                     "tol_1e-15_max_abs_error_vs_prpack_below_1e-13": max(abs(tight[v] - ref[v]) for v in range(n)) < 1e-13},
        "igraph": "PRPACK solves the linear system directly",
        "everygraph": "the oracle compares against the fixed point; tolerance in oracle/tolerances.json",
    })
    return rows


def main(write: bool) -> int:
    rows = cases()
    for r in rows:
        r["agree"] = r["networkx"] == r["igraph"]
    doc = {
        "format": "everygraph-oracle-semantics-probe",
        "version": 1,
        "libraries": {"networkx": nx.__version__, "igraph": ig.__version__},
        "rounding": "reals rounded to 6 decimals",
        "cases": rows,
    }
    text = json.dumps(doc, indent=1) + "\n"
    if write:
        OUT.parent.mkdir(parents=True, exist_ok=True)
        OUT.write_text(text)
    else:
        old = json.loads(OUT.read_text())
        if old["cases"] != json.loads(text)["cases"]:
            changed = [r["id"] for r, o in zip(rows, old["cases"]) if json.loads(json.dumps(r)) != o]
            print(f"semantics probe changed: {changed}")
            return 1
    for r in rows:
        mark = "agree" if r["agree"] else "DIFFER"
        print(f"{r['id']:28} {mark:6} nx={r['networkx']}  ig={r['igraph']}  eg={r['everygraph']}")
    return 0
