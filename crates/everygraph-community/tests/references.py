# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Reference values for tests/leiden.rs and tests/weights.rs, from python-igraph, leidenalg
and NetworkX (the oracle's pinned versions). Writes tests/references.json; the Rust tests
read it and build the same graphs with the same generators.

    cd oracle && uv run --locked python ../crates/everygraph-community/tests/references.py

For each graph and each method: the best and the mean modularity over seeds 0-9, at
resolution 1. igraph's Leiden runs until an iteration changes nothing (n_iterations=-1);
igraph's multilevel and NetworkX's Louvain run to their own end. Then weighted graphs
with loops (#58): igraph's Leiden for modularity at several resolutions, leidenalg's for
CPM, the values of fixed partitions; and directed graphs, with leidenalg's Leiden for
Leicht-Newman modularity and igraph's directed and projected values.
"""

import json
import math
import pathlib
import random
import statistics

import igraph
import leidenalg
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


def unit(rng):
    """A float in [0, 1) from the top 53 bits of a draw."""
    return (rng.next() >> 11) / 2**53


def weighted_planted(blocks, size, p_in, p_out, seed, directed=False):
    """As `planted`, every pair (every ordered pair when directed) drawn in order; each
    edge then draws its weight, 1 to 3 inside a block and 0.1 to 0.5 between blocks;
    then a loop of weight 0.5 to 1.5 at every seventh node."""
    rng = SplitMix64(seed)
    n = blocks * size
    edges, weights = [], []
    for u in range(n):
        for v in range(n) if directed else range(u + 1, n):
            if u == v:
                continue
            inside = u // size == v // size
            if rng.next() % 1000 < (p_in if inside else p_out):
                edges.append((u, v))
                weights.append(1.0 + 2.0 * unit(rng) if inside else 0.1 + 0.4 * unit(rng))
    for v in range(0, n, 7):
        edges.append((v, v))
        weights.append(0.5 + unit(rng))
    return n, edges, weights


def weighted(n, edges, seed):
    """`edges` with a weight of 0.5 to 4.5 each."""
    rng = SplitMix64(seed)
    return n, edges, [0.5 + 4.0 * unit(rng) for _ in edges]


def cpm(g, membership, weights, gamma):
    """The Constant Potts Model in the binomial form of Traag, Van Dooren and Nesterov
    (2011), Everygraph's: sum over communities of the weight inside, loops once, less
    gamma times its pairs of nodes. Read from leidenalg's CPMVertexPartition, whose
    quality() is twice that, less 2·gamma·n when the graph has a loop (its possible
    edges are then n_c·(n_c + 1)/2); checked here on every partition it scores."""
    p = leidenalg.CPMVertexPartition(g, initial_membership=membership, weights=weights,
                                     resolution_parameter=gamma)
    loops = any(e.is_loop() for e in g.es)
    value = p.quality() / 2 + (gamma * g.vcount() if loops else 0.0)
    inside = sum(w for e, w in zip(g.es, weights) if membership[e.source] == membership[e.target])
    sizes = {}
    for c in membership:
        sizes[c] = sizes.get(c, 0) + 1
    binomial = inside - gamma * sum(s * (s - 1) / 2 for s in sizes.values())
    assert abs(value - binomial) <= 1e-9 * max(1.0, abs(binomial)), (value, binomial)
    return value


# Name: (graph, resolutions of modularity, resolution of CPM, block size).
WEIGHTED = {
    "w-karate": (weighted(*karate(), 11), [1.0], 0.5, 17),
    "w-ring-30x5": (weighted(*ring(30, 5), 12), [1.0, 2.0], 1.0, 5),
    "w-planted-8x100-120-4": (weighted_planted(8, 100, 120, 4, 1), [1.0, 0.5], 0.1, 100),
    "w-planted-20x100-80-8": (weighted_planted(20, 100, 80, 8, 3), [1.0], 0.05, 100),
}


def fixed(n, block):
    """Two partitions: `v % 5`, and blocks of `block` consecutive nodes."""
    return {"mod5": [v % 5 for v in range(n)], "blocks": [v // block for v in range(n)]}


def weighted_references():
    """For each weighted graph, with loops: igraph's Leiden over seeds 0-9 for
    modularity at each resolution and for CPM, scored by igraph's modularity and by
    `cpm`; and the values of the partitions of `fixed`."""
    out = {}
    for name, ((n, edges, w), gammas, gamma_cpm, block) in WEIGHTED.items():
        g = igraph.Graph(n=n, edges=edges)
        r = {"nodes": n, "edges": len(edges), "weight_sum": math.fsum(w), "block": block,
             "modularity": {}}
        for gamma in gammas:
            values = []
            for seed in range(10):
                random.seed(seed)
                m = g.community_leiden(objective_function="modularity", weights=w,
                                       resolution=gamma, n_iterations=-1).membership
                values.append(g.modularity(m, weights=w, resolution=gamma))
            r["modularity"][str(gamma)] = summary(values)
        values = []
        for seed in range(10):
            p = leidenalg.find_partition(g, leidenalg.CPMVertexPartition, weights=w,
                                         resolution_parameter=gamma_cpm, n_iterations=-1,
                                         seed=seed)
            values.append(cpm(g, p.membership, w, gamma_cpm))
        r["cpm"] = {"resolution": gamma_cpm, **summary(values)}
        r["values"] = {
            key: {
                "modularity": {str(x): g.modularity(m, weights=w, resolution=x) for x in gammas},
                "cpm": cpm(g, m, w, gamma_cpm),
            }
            for key, m in fixed(n, block).items()
        }
        out[name] = r
    return out


def flow(blocks, size, p_in, p_out, forward, back, seed):
    """Arcs inside a block with probability p_in / 1000 and between blocks p_out / 1000,
    every ordered pair drawn in order; an arc from a lower to a higher id weighs
    `forward`, the other way `back`, each times a draw in [1, 2)."""
    rng = SplitMix64(seed)
    n = blocks * size
    arcs, weights = [], []
    for u in range(n):
        for v in range(n):
            if u == v:
                continue
            if rng.next() % 1000 < (p_in if u // size == v // size else p_out):
                arcs.append((u, v))
                weights.append((forward if u < v else back) * (1.0 + unit(rng)))
    return n, arcs, weights


def leidenalg_directed(g, w):
    """The Leicht-Newman modularity of leidenalg's ModularityVertexPartition, which
    reads direction, over seeds 0-9."""
    values = []
    for seed in range(10):
        p = leidenalg.find_partition(g, leidenalg.ModularityVertexPartition, weights=w,
                                     n_iterations=-1, seed=seed)
        values.append(g.modularity(p.membership, weights=w, directed=True))
    return summary(values)


def directed_references():
    """Directed graphs: a planted one with loops, with the Leicht-Newman and projected
    modularity of the partitions of `fixed`; and one whose arcs run mostly from lower to
    higher ids, where direction changes the best partition. For both, leidenalg's best
    Leicht-Newman value."""
    n, edges, w = weighted_planted(8, 60, 150, 6, 5, directed=True)
    g = igraph.Graph(n=n, edges=edges, directed=True)
    fn, farcs, fw = flow(4, 50, 200, 100, 8.0, 0.1, 100)
    f = igraph.Graph(n=fn, edges=farcs, directed=True)
    return {
        "nodes": n,
        "edges": len(edges),
        "weight_sum": math.fsum(w),
        "block": 60,
        "leidenalg": leidenalg_directed(g, w),
        "values": {
            key: {
                "directed": g.modularity(m, weights=w, directed=True),
                "undirected": g.modularity(m, weights=w, directed=False),
                "directed_half": g.modularity(m, weights=w, resolution=0.5, directed=True),
            }
            for key, m in fixed(n, 60).items()
        },
        "flow": {
            "nodes": fn,
            "edges": len(farcs),
            "weight_sum": math.fsum(fw),
            "leidenalg": leidenalg_directed(f, fw),
        },
    }


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
    versions = {"igraph": igraph.__version__, "leidenalg": leidenalg.__version__,
                "networkx": nx.__version__}
    everything = {
        "versions": versions,
        "graphs": out,
        "weighted": weighted_references(),
        "directed": directed_references(),
    }
    text = json.dumps(everything, indent=1, sort_keys=True)
    (HERE / "references.json").write_text(text + "\n")


if __name__ == "__main__":
    main()
