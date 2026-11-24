# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy", "pyarrow"]
# ///
import sys, time, numpy as np, pyarrow as pa, pyarrow.compute, glabpy
g = glabpy.Graph.open(sys.argv[1])
t = time.perf_counter(); col, it = g.pagerank_arrow(max_iter=20, tol=0.0); dt = time.perf_counter() - t
a = pa.array(col)
print({"python": sys.version.split()[0], "n": g.n, "pr20_s": round(dt, 3), "sum": round(pa.compute.sum(a).as_py(), 6),
       "zero_copy": a.buffers()[1].address == col.buffer_addresses()[-1], "deg0": int(g.degree()[0])})
