# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Reference modularities for tests/leiden.rs, from python-igraph and NetworkX (the oracle's
pinned versions). Writes tests/references.json; the Rust test reads it and builds the same
graphs with the same generator.

    cd oracle && uv run --locked python ../crates/everygraph-community/tests/references.py

For each graph and each method: the best and the mean modularity over seeds 0-9, at
resolution 1. igraph's Leiden runs until an iteration changes nothing (n_iterations=-1);
igraph's multilevel and NetworkX's Louvain run to their own end.
"""

import json
import pathlib
import random
import statistics

import igraph
import networkx as nx

MASK = (1 << 64) - 1
HERE = pathlib.Path(__file__).resolve().parent


class SplitMix64:
    def __init__(self, seed):
        self.state = seed & MASK

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)


def planted(blocks, size, p_in, p_out, seed):
    """Every pair u < v is an edge with probability p_in / 1000 inside a block and
    p_out / 1000 between blocks: one draw per pair, in order."""
    rng = SplitMix64(seed)
    n = blocks * size
    edges = []
    for u in range(n):
        for v in range(u + 1, n):
            p = p_in if u // size == v // size else p_out
            if rng.next() % 1000 < p:
                edges.append((u, v))
    return n, edges


def ring(cliques, size):
    """`cliques` cliques of `size` nodes; clique i's last node joined to clique i+1's first."""
    edges = []
    for c in range(cliques):
        base = c * size
        edges += [(base + a, base + b) for a in range(size) for b in range(a + 1, size)]
        edges.append((base + size - 1, ((c + 1) % cliques) * size))
    return cliques * size, edges


def karate():
    rows = (HERE / "../../../oracle/golden/karate.tsv").read_text().splitlines()
    edges = [tuple(map(int, r.split())) for r in rows if r and not r.startswith("#")]
    return 34, edges


GRAPHS = {
    "karate": karate(),
    "ring-30x5": ring(30, 5),
    "planted-8x100-120-4": planted(8, 100, 120, 4, 1),
    "planted-50x40-300-3": planted(50, 40, 300, 3, 2),
    "planted-20x100-80-8": planted(20, 100, 80, 8, 3),
}


def summary(values):
    return {"best": max(values), "mean": statistics.fmean(values)}


def main():
    out = {}
    for name, (n, edges) in GRAPHS.items():
        g = igraph.Graph(n=n, edges=edges)
        h = nx.Graph()
        h.add_nodes_from(range(n))
        h.add_edges_from(edges)
        leiden, multilevel, louvain = [], [], []
        for seed in range(10):
            random.seed(seed)
            leiden.append(g.community_leiden(objective_function="modularity", n_iterations=-1).modularity)
            random.seed(seed)
            multilevel.append(g.community_multilevel().modularity)
            parts = nx.community.louvain_communities(h, seed=seed)
            louvain.append(nx.community.modularity(h, parts))
        out[name] = {
            "nodes": n,
            "edges": len(edges),
            "igraph_leiden": summary(leiden),
            "igraph_multilevel": summary(multilevel),
            "networkx_louvain": summary(louvain),
        }
    versions = {"igraph": igraph.__version__, "networkx": nx.__version__}
    text = json.dumps({"versions": versions, "graphs": out}, indent=1, sort_keys=True)
    (HERE / "references.json").write_text(text + "\n")


if __name__ == "__main__":
    main()
