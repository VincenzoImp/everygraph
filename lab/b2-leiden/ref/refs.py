#!/usr/bin/env python
"""Reference partitions: igraph multilevel / leiden, NetworKit PLM.
Writes membership as little-endian u32 to ref/out/<graph>.<algo>.u32 and timings to ref/out/<graph>.<algo>.json.
Modularity is computed later by the common Rust routine (b2 eval)."""
import sys, time, json, os
import numpy as np

def load(name):
    d = f"/home/tux/graphlab/data/csr/{name}"
    meta = json.load(open(f"{d}/meta.json"))
    off = np.fromfile(f"{d}/offsets.u64", dtype=np.uint64)
    tgt = np.fromfile(f"{d}/targets.u32", dtype=np.uint32)
    n = meta["n"]
    deg = np.diff(off).astype(np.int64)
    src = np.repeat(np.arange(n, dtype=np.uint32), deg)
    mask = src < tgt
    return n, src[mask], tgt[mask]

def save(name, algo, memb, secs, extra=None):
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")
    np.asarray(memb, dtype=np.uint32).tofile(f"{out}/{name}.{algo}.u32.tmp")
    os.replace(f"{out}/{name}.{algo}.u32.tmp", f"{out}/{name}.{algo}.u32")
    j = {"graph": name, "algo": algo, "seconds": secs}
    if extra: j.update(extra)
    json.dump(j, open(f"{out}/{name}.{algo}.json", "w"))
    print(j, flush=True)

def main():
    name = sys.argv[1]; algos = sys.argv[2].split(",")
    n, s, t = load(name)
    print("loaded", name, n, len(s), flush=True)
    if any(a.startswith("nk") for a in algos):
        import networkit as nk
        nk.setNumberOfThreads(int(os.environ.get("NK_THREADS", "6")))
        g = nk.GraphFromCoo((np.ones(len(s)), (s.astype(np.int64), t.astype(np.int64))), n=n, weighted=False, directed=False)
        print("nk graph", g.numberOfNodes(), g.numberOfEdges(), flush=True)
        if "nk_plm" in algos:
            t0 = time.time(); plm = nk.community.PLM(g, refine=False, gamma=1.0); plm.run(); dt = time.time() - t0
            save(name, "nk_plm", np.array(plm.getPartition().getVector()), dt, {"threads": nk.getCurrentNumberOfThreads()})
        if "nk_plmr" in algos:
            t0 = time.time(); plm = nk.community.PLM(g, refine=True, gamma=1.0); plm.run(); dt = time.time() - t0
            save(name, "nk_plmr", np.array(plm.getPartition().getVector()), dt, {"threads": nk.getCurrentNumberOfThreads()})
        if "nk_leiden" in algos:
            t0 = time.time(); a = nk.community.ParallelLeiden(g, iterations=3, randomize=False, gamma=1.0); a.run(); dt = time.time() - t0
            save(name, "nk_leiden", np.array(a.getPartition().getVector()), dt, {"threads": nk.getCurrentNumberOfThreads()})
        if "nk_leiden_rand" in algos:
            t0 = time.time(); a = nk.community.ParallelLeiden(g); a.run(); dt = time.time() - t0   # defaults: iterations=3, randomize=True
            save(name, "nk_leiden_rand", np.array(a.getPartition().getVector()), dt, {"threads": int(os.environ.get("NK_THREADS", "6"))})
        del g
    if any(a.startswith("ig") for a in algos):
        import igraph as ig
        e = np.empty((len(s), 2), dtype=np.int64); e[:, 0] = s; e[:, 1] = t
        g = ig.Graph(n=n, edges=e, directed=False); del e
        print("igraph", g.vcount(), g.ecount(), flush=True)
        if "ig_multilevel" in algos:
            t0 = time.time(); c = g.community_multilevel(); dt = time.time() - t0
            save(name, "ig_multilevel", c.membership, dt, {"threads": 1, "ig_modularity": c.modularity})
        if "ig_leiden" in algos:
            t0 = time.time(); c = g.community_leiden(objective_function="modularity", resolution=1.0, n_iterations=2); dt = time.time() - t0
            save(name, "ig_leiden", c.membership, dt, {"threads": 1, "n_iterations": 2})
        if "ig_leiden_conv" in algos:
            t0 = time.time(); c = g.community_leiden(objective_function="modularity", resolution=1.0, n_iterations=10); dt = time.time() - t0
            save(name, "ig_leiden_conv", c.membership, dt, {"threads": 1, "n_iterations": 10})
main()
