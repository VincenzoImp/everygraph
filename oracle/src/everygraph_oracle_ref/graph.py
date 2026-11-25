# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The exchange form (graph.json + edges.tsv + partition.txt) and the analytic graph.

The projection is written here independently of the Rust one
(crates/everygraph-oracle/src/projection.rs), with plain dictionaries, so that the
two can disagree if either is wrong. Semantics (plan/issues/m2-09-graph-semantics.md):
self-loops dropped unless the descriptor keeps them (a kept loop counts once per node);
parallel rows analysed once per pair with multiplicity, summed weight (in row order)
and smallest weight; undirected projection of a directed graph = pair {u, v} present if
either arc is.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path

GRAPH_FORMAT = "everygraph-oracle-graph"
VERSION = 1


@dataclass
class Graph:
    name: str
    n: int
    directed: bool
    src: list[int]
    dst: list[int]
    weight: list[float] | None
    time: list[int] | None
    partition: list[int] | None
    keep_loops: bool
    kernels: list[dict]
    provenance: dict = field(default_factory=dict)


def read_entry(d: Path) -> Graph:
    desc = json.loads((d / "graph.json").read_text())
    if desc["format"] != GRAPH_FORMAT or desc["version"] != VERSION:
        raise ValueError(f"{d}: format {desc['format']} v{desc['version']}")
    cols = desc["columns"]
    wi = cols.index("weight") if "weight" in cols else None
    ti = cols.index("time") if "time" in cols else None
    src, dst = [], []
    weight = [] if wi is not None else None
    time = [] if ti is not None else None
    with (d / "edges.tsv").open() as f:
        for line in f:
            if not line.strip() or line.startswith("#"):
                continue
            parts = line.rstrip("\n").split("\t")
            src.append(int(parts[0]))
            dst.append(int(parts[1]))
            if weight is not None:
                weight.append(float(parts[wi]))
            if time is not None:
                time.append(int(parts[ti]))
    if len(src) != desc["rows"]:
        raise ValueError(f"{d}: {len(src)} rows, descriptor says {desc['rows']}")
    partition = None
    if desc.get("partition"):
        partition = [int(x) for x in (d / desc["partition"]).read_text().split()]
    return Graph(
        name=desc["name"],
        n=desc["n"],
        directed=desc["directed"],
        src=src,
        dst=dst,
        weight=weight,
        time=time,
        partition=partition,
        keep_loops=desc["semantics"]["self_loops"] == "keep",
        kernels=desc["kernels"],
        provenance=desc.get("provenance") or {},
    )


@dataclass
class Agg:
    mult: int
    wsum: float
    wmin: float


@dataclass
class Projection:
    n: int
    directed: bool
    arcs: dict[tuple[int, int], Agg]  # directed: (u, v); undirected: (min, max)
    undirected: dict[tuple[int, int], Agg]  # (min, max), both directions aggregated
    loop_nodes: list[int]  # nodes with at least one loop row
    keep_loops: bool
    self_loop_rows: int
    duplicate_rows: int


def _add(d: dict, key, w: float) -> bool:
    a = d.get(key)
    if a is None:
        d[key] = Agg(1, w, w)
        return True
    a.mult += 1
    a.wsum += w
    a.wmin = min(a.wmin, w)
    return False


def project(g: Graph) -> Projection:
    arcs: dict = {}
    und: dict = {}
    loops: set[int] = set()
    self_loop_rows = duplicates = 0
    for i, (u, v) in enumerate(zip(g.src, g.dst)):
        w = g.weight[i] if g.weight is not None else 1.0
        if u == v:
            loops.add(u)
            self_loop_rows += 1
            continue
        key = (u, v) if g.directed else (min(u, v), max(u, v))
        if not _add(arcs, key, w):
            duplicates += 1
        if g.directed:
            _add(und, (min(u, v), max(u, v)), w)
    if not g.directed:
        und = arcs
    return Projection(
        n=g.n,
        directed=g.directed,
        arcs=arcs,
        undirected=und,
        loop_nodes=sorted(loops),
        keep_loops=g.keep_loops,
        self_loop_rows=self_loop_rows,
        duplicate_rows=duplicates,
    )


def to_networkx(p: Projection, *, undirected: bool = False, loops: bool = False):
    import networkx as nx

    directed = p.directed and not undirected
    G = nx.DiGraph() if directed else nx.Graph()
    G.add_nodes_from(range(p.n))
    pairs = p.arcs if directed else p.undirected
    for (u, v), a in pairs.items():
        G.add_edge(u, v, mult=a.mult, wsum=a.wsum, wmin=a.wmin)
    if loops:
        for v in p.loop_nodes:
            G.add_edge(v, v, mult=1, wsum=1.0, wmin=1.0)
    return G


def to_igraph(p: Projection, *, undirected: bool = False, loops: bool = False):
    import igraph as ig

    directed = p.directed and not undirected
    pairs = p.arcs if directed else p.undirected
    edges = list(pairs.keys())
    aggs = list(pairs.values())
    if loops:
        edges += [(v, v) for v in p.loop_nodes]
        aggs += [Agg(1, 1.0, 1.0) for _ in p.loop_nodes]
    H = ig.Graph(n=p.n, edges=edges, directed=directed)
    H.es["mult"] = [a.mult for a in aggs]
    H.es["wsum"] = [a.wsum for a in aggs]
    H.es["wmin"] = [a.wmin for a in aggs]
    return H


def to_scipy(p: Projection, *, undirected: bool = False, weight: str | None = None):
    """Sparse adjacency; symmetric when undirected. `weight` is None (1s), 'wsum' or 'wmin'."""
    import numpy as np
    import scipy.sparse as sp

    directed = p.directed and not undirected
    pairs = p.arcs if directed else p.undirected
    rows, cols, vals = [], [], []
    for (u, v), a in pairs.items():
        w = 1.0 if weight is None else getattr(a, weight)
        rows.append(u)
        cols.append(v)
        vals.append(w)
        if not directed:
            rows.append(v)
            cols.append(u)
            vals.append(w)
    return sp.csr_array(
        (np.array(vals, dtype=np.float64), (np.array(rows, dtype=np.int64), np.array(cols, dtype=np.int64))),
        shape=(p.n, p.n),
    )
