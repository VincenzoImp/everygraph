import itertools
import numpy as np
import networkx as nx
from glabpy._glab import Graph

__all__ = ["GlabGraph", "BackendInterface"]


class GlabGraph:
    """Backend graph object: immutable symmetric CSR + optional node-label array.

    labels is None  -> nodes are the ints 0..n-1 (no Python objects at all).
    """
    __networkx_backend__ = "glab"

    def __init__(self, g: Graph, labels=None, graph_attrs=None):
        self._g = g
        self._labels = labels  # python list or None
        self.graph = dict(graph_attrs or {})
        self.__networkx_cache__ = {}

    # minimal duck-typing expected by the dispatcher / users
    def is_directed(self):
        return False

    def is_multigraph(self):
        return False

    def number_of_nodes(self):
        return self._g.n

    __len__ = number_of_nodes

    def number_of_edges(self):
        return self._g.entries // 2

    def nodes_list(self):
        return list(range(self._g.n)) if self._labels is None else self._labels

    # ---- native constructors (no nx.Graph ever built) ----
    @classmethod
    def from_csr(cls, path):
        return cls(Graph.open(path))

    @classmethod
    def from_arrays(cls, src, dst, n=None, threads=None):
        src = np.ascontiguousarray(src, dtype=np.uint32)
        dst = np.ascontiguousarray(dst, dtype=np.uint32)
        if n is None:
            n = int(max(src.max(), dst.max())) + 1
        return cls(Graph.from_edges(src, dst, n, threads))

    @classmethod
    def read_edgelist(cls, path, threads=None, compact=True):
        """Read a 2-column int edge list (CSV with header src,dst, or SNAP text) straight into CSR."""
        import pyarrow.csv as pc
        if path.endswith(".csv"):
            t = pc.read_csv(path, convert_options=pc.ConvertOptions(column_types={"src": "uint32", "dst": "uint32"}))
        else:
            t = pc.read_csv(path, read_options=pc.ReadOptions(column_names=["src", "dst"], skip_rows=0),
                            parse_options=pc.ParseOptions(delimiter="\t", ignore_empty_lines=True,
                                                          invalid_row_handler=lambda r: "skip"),
                            convert_options=pc.ConvertOptions(column_types={"src": "uint32", "dst": "uint32"}))
        src = t.column("src").to_numpy()
        dst = t.column("dst").to_numpy()
        if compact:
            # NetworkX semantics: only ids that appear are nodes; keep first-appearance order? (nx uses
            # insertion order; we use sorted order -> documented difference)
            ids, inv = np.unique(np.concatenate([src, dst]), return_inverse=True)
            inv = inv.astype(np.uint32)
            g = cls.from_arrays(inv[: len(src)], inv[len(src):], n=len(ids), threads=threads)
            if not (len(ids) and ids[0] == 0 and ids[-1] == len(ids) - 1):
                g._labels = ids.tolist()
            return g
        return cls.from_arrays(src, dst, threads=threads)


def _unsupported_pr_kwargs(kwargs):
    return any(kwargs.get(k) is not None for k in ("personalization", "nstart", "dangling"))


class BackendInterface:
    # ---------------- conversion ----------------
    @staticmethod
    def convert_from_nx(G, edge_attrs=None, node_attrs=None, preserve_edge_attrs=False,
                        preserve_node_attrs=False, preserve_graph_attrs=False, name=None,
                        graph_name=None):
        if G.is_directed() or G.is_multigraph():
            raise NotImplementedError("glab prototype: only undirected simple graphs")
        # Attribute semantics: we only support unweighted algorithms.  If an algorithm asked for an
        # edge attribute (e.g. pagerank weight='weight') and any edge actually carries it, refuse,
        # so the dispatcher falls back to NetworkX instead of silently returning wrong numbers.
        if preserve_edge_attrs:
            raise NotImplementedError("edge attributes not supported")
        adj = G._adj
        chain = itertools.chain.from_iterable
        if edge_attrs:
            keys = [k for k in edge_attrs if k is not None]
            # C-speed pre-check: does any edge carry any attribute at all?
            if keys and any(map(len, chain(map(dict.values, adj.values())))):
                for _, _, d in G.edges(data=True):
                    if d and any(k in d for k in keys):
                        raise NotImplementedError(f"edge attribute(s) {keys} present; unsupported")
        nodes = list(adj)
        n = len(nodes)
        deg = np.fromiter(map(len, adj.values()), dtype=np.int64, count=n)
        m2 = int(deg.sum())
        identity = False
        try:
            if set(map(type, nodes)) - {int}:
                raise TypeError("non-int labels")
            keys_arr = np.fromiter(nodes, dtype=np.int64, count=n)  # fast path: integer labels
            raw = np.fromiter(chain(adj.values()), dtype=np.int64, count=m2)
            identity = bool(n == 0 or (keys_arr[0] == 0 and np.all(np.diff(keys_arr) == 1)))
            if identity:
                dst = raw.astype(np.uint32)
            else:
                order = np.argsort(keys_arr)
                dst = order[np.searchsorted(keys_arr, raw, sorter=order)].astype(np.uint32)
        except (TypeError, ValueError, OverflowError):  # arbitrary hashable labels
            idx = {u: i for i, u in enumerate(nodes)}
            dst = np.fromiter((idx[v] for v in chain(adj.values())), dtype=np.uint32, count=m2)
        src = np.repeat(np.arange(n, dtype=np.uint32), deg)
        g = Graph.from_edges(src, dst, n)
        return GlabGraph(g, None if identity else nodes, G.graph if preserve_graph_attrs else None)

    @staticmethod
    def convert_to_nx(obj, *, name=None):
        if isinstance(obj, GlabGraph):
            G = nx.Graph()
            labels = obj.nodes_list()
            G.add_nodes_from(labels)
            off = obj._g.offsets_view()
            tg = obj._g.targets_view()
            src = np.repeat(np.arange(obj._g.n), np.diff(off).astype(np.int64))
            mask = src < tg
            s, d = src[mask].tolist(), tg[mask].tolist()
            if obj._labels is None:
                G.add_edges_from(zip(s, d))
            else:
                G.add_edges_from((labels[a], labels[b]) for a, b in zip(s, d))
            G.graph.update(obj.graph)
            return G
        return obj

    # ---------------- dispatch hooks ----------------
    @staticmethod
    def can_run(name, args, kwargs):
        if name == "pagerank":
            if _unsupported_pr_kwargs(kwargs):
                return "personalization/nstart/dangling not supported"
            return True
        return name in ("connected_components", "number_connected_components")

    @staticmethod
    def should_run(name, args, kwargs):
        G = args[0] if args else kwargs.get("G")
        if hasattr(G, "number_of_edges") and not isinstance(G, GlabGraph) and G.number_of_edges() < 1000:
            return "graph too small to be worth converting"
        return True

    # ---------------- algorithms ----------------
    @staticmethod
    def pagerank(G, alpha=0.85, personalization=None, max_iter=100, tol=1.0e-6, nstart=None,
                 weight="weight", dangling=None, *, threads=None):
        if _unsupported_pr_kwargs(dict(personalization=personalization, nstart=nstart, dangling=dangling)):
            raise NotImplementedError
        if len(G) == 0:
            return {}
        x, iters, _ = G._g.pagerank(alpha, max_iter, tol, threads)
        if iters >= max_iter:
            # NetworkX raises if not converged within max_iter; mirror it
            raise nx.PowerIterationFailedConvergence(max_iter)
        return dict(zip(G.nodes_list(), x.tolist()))

    @staticmethod
    def connected_components(G, *, threads=None):
        if G.is_directed():
            raise nx.NetworkXNotImplemented("not implemented for directed type")
        lab = G._g.wcc(threads)
        order = np.argsort(lab, kind="stable")
        sl = lab[order]
        cuts = np.flatnonzero(np.diff(sl)) + 1
        labels = G._labels
        # components ordered by smallest node position == NetworkX discovery order
        for part in np.split(order, cuts):
            if labels is None:
                yield set(part.tolist())
            else:
                yield {labels[i] for i in part.tolist()}

    @staticmethod
    def number_connected_components(G, *, threads=None):
        lab = G._g.wcc(threads)
        return int(np.count_nonzero(lab == np.arange(len(lab), dtype=lab.dtype)))
