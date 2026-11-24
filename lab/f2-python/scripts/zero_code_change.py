# Plain NetworkX user code: no mention of any backend.
import logging, sys, time, networkx as nx
logging.basicConfig(level=logging.WARNING)
logging.getLogger("networkx").setLevel(logging.DEBUG if "-v" in sys.argv else logging.WARNING)
G = nx.gnm_random_graph(200_000, 1_000_000, seed=1)
t = time.perf_counter(); pr = nx.pagerank(G); t1 = time.perf_counter() - t
t = time.perf_counter(); cc = list(nx.connected_components(G)); t2 = time.perf_counter() - t
print(f"priority={nx.config.backend_priority.algos} pagerank {t1:.3f}s  cc {t2:.3f}s  ncc={len(cc)}  pr[0]={pr[0]:.3e}")
