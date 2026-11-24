"""Goal C: NetworkX backend — pure nx vs backend (with conversion, cached) vs native backend graph."""
import json, os, sys, time, resource, gc, logging
import numpy as np, pyarrow.csv as pcsv
import networkx as nx
nx.config.warnings_to_ignore.add('cache')
from glabpy.nx_backend import GlabGraph

path = sys.argv[1]; out = sys.argv[2]
skip_nx_cc = os.environ.get("SKIP_NX_CC") == "1"
R = {"edges_csv": path, "networkx": nx.__version__, "rayon_threads": int(os.environ.get("RAYON_NUM_THREADS", "0") or 0)}
def tm(f):
    t0 = time.perf_counter(); r = f(); return r, time.perf_counter() - t0
def rss(): return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024

# --- build an nx.Graph the way a user would (from the CSV) ---
def build_nx():
    t = pcsv.read_csv(path)
    G = nx.Graph()
    G.add_edges_from(zip(t.column("src").to_pylist(), t.column("dst").to_pylist()))
    return G
G, R["nx_build_from_csv_s"] = tm(build_nx)
R["n"], R["m"] = G.number_of_nodes(), G.number_of_edges()
R["rss_after_nx_build_mb"] = rss()
nx.pagerank(nx.path_graph(2000)); nx.pagerank(nx.path_graph(2000), backend='glab')  # warm imports
nx.config.cache_converted_graphs = False  # measure conversion every call first

# --- PageRank ---
pr_nx, R["nx_pagerank_default_s"] = tm(lambda: nx.pagerank(G))
pr_nx_t, R["nx_pagerank_tol1e-10_s"] = tm(lambda: nx.pagerank(G, tol=1e-10, max_iter=1000))
conv, R["convert_from_nx_s"] = tm(lambda: nx.utils.backends._load_backend("glab").convert_from_nx(G, edge_attrs={"weight": 1}))
pr_b, R["backend_pagerank_default_incl_conversion_s"] = tm(lambda: nx.pagerank(G, backend="glab"))
pr_bt, R["backend_pagerank_tol1e-10_incl_conversion_s"] = tm(lambda: nx.pagerank(G, tol=1e-10, max_iter=1000, backend="glab"))
_, R["backend_pagerank_tol1e-10_on_converted_graph_s"] = tm(lambda: nx.pagerank(conv, tol=1e-10, max_iter=1000))
nx.config.cache_converted_graphs = True
G.__networkx_cache__.clear()
_, R["backend_pagerank_tol1e-10_cache_first_s"] = tm(lambda: nx.pagerank(G, tol=1e-10, max_iter=1000, backend="glab"))
_, R["backend_pagerank_tol1e-10_cache_hit_s"] = tm(lambda: nx.pagerank(G, tol=1e-10, max_iter=1000, backend="glab"))
nx.config.cache_converted_graphs = False
def maxdiff(a, b): return max(abs(a[k] - b[k]) for k in a)
R["pagerank_maxabs_diff_default"] = maxdiff(pr_nx, pr_b)
R["pagerank_maxabs_diff_tol1e-10"] = maxdiff(pr_nx_t, pr_bt)
R["pagerank_same_key_order"] = list(pr_nx) == list(pr_b)
# how many iterations does nx's default tol actually do? (tol is compared to N*tol in L1)
_, it_default, _ = conv._g.pagerank(0.85, 100, 1e-6)
_, it_tight, _ = conv._g.pagerank(0.85, 1000, 1e-10)
R["pagerank_iterations_default_tol"] = it_default
R["pagerank_iterations_tol1e-10"] = it_tight

# --- connected components ---
if not skip_nx_cc:
    cc_nx, R["nx_connected_components_s"] = tm(lambda: list(nx.connected_components(G)))
cc_b, R["backend_cc_incl_conversion_s"] = tm(lambda: list(nx.connected_components(G, backend="glab")))
_, R["backend_cc_on_converted_graph_s"] = tm(lambda: list(nx.connected_components(conv)))
_, R["backend_number_cc_on_converted_graph_s"] = tm(lambda: nx.number_connected_components(conv))
_, R["backend_wcc_kernel_only_s"] = tm(lambda: conv._g.wcc())
R["n_components"] = len(cc_b)
if not skip_nx_cc:
    R["cc_identical_incl_order"] = cc_nx == cc_b
R["rss_peak_mb"] = rss()
del G, conv; gc.collect()

# --- native path: edge list -> backend graph, no nx.Graph at all ---
Gb, R["native_read_edgelist_to_csr_s"] = tm(lambda: GlabGraph.read_edgelist(path))
_, R["native_nx_pagerank_tol1e-10_s"] = tm(lambda: nx.pagerank(Gb, tol=1e-10, max_iter=1000))
_, R["native_nx_pagerank_default_s"] = tm(lambda: nx.pagerank(Gb))
_, R["native_cc_list_s"] = tm(lambda: list(nx.connected_components(Gb)))
_, R["native_number_cc_s"] = tm(lambda: nx.number_connected_components(Gb))
R["native_note"] = "native graph: ids compacted with np.unique (nodes = ids present, sorted order, unlike nx insertion order)"
R["native_n"] = Gb.number_of_nodes()
txt = json.dumps(R, indent=1); print(txt); open(out, "w").write(txt)
