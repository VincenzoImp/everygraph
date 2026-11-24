"""Independent spot check: sample raw text edges, look them up in a glab-csr-v0 dir (both directions)."""
import sys, os, numpy as np
txt, d = sys.argv[1], sys.argv[2]
ids = np.fromfile(f"{d}/ids.u64", dtype=np.uint64)
off = np.memmap(f"{d}/offsets.u64", dtype=np.uint64, mode="r")
tg = np.memmap(f"{d}/targets.u32", dtype=np.uint32, mode="r")
size = os.path.getsize(txt)
rng = np.random.default_rng(1)
A, B = [], []
with open(txt, "rb") as f:
    for pos in [0, max(0, size - (1 << 18))] + [int(x) for x in rng.integers(0, max(1, size - (1 << 18)), 40)]:
        f.seek(pos); blk = f.read(1 << 18)
        lines = blk.split(b"\n")[1:-1]
        for ln in lines[:: max(1, len(lines) // 250)]:
            if not ln or ln.startswith(b"#"): continue
            a, b = ln.split()
            A.append(int(a)); B.append(int(b))
A = np.array(A, dtype=np.uint64); B = np.array(B, dtype=np.uint64)
U = np.searchsorted(ids, A); V = np.searchsorted(ids, B)
assert (ids[U] == A).all() and (ids[V] == B).all()
checked = missing = 0
for u, v in zip(U.tolist(), V.tolist()):
    if u == v: continue
    for x, y in ((u, v), (v, u)):
        adj = np.asarray(tg[int(off[x]): int(off[x + 1])])
        k = int(np.searchsorted(adj, np.uint32(y)))
        checked += 1
        if k >= len(adj) or adj[k] != y: missing += 1
print("spotcheck", d, "pairs_checked", checked, "missing", missing, "n", len(ids), "entries", len(tg), "deg_sum_ok", int(off[-1]) == len(tg))
sys.exit(1 if missing else 0)
