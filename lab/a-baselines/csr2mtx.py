import sys, json, numpy as np, pyarrow as pa, pyarrow.csv as pc
g = sys.argv[1]; D = f"/home/tux/graphlab/data/csr/{g}"; n = json.load(open(f"{D}/meta.json"))["n"]
off = np.fromfile(f"{D}/offsets.u64", dtype=np.uint64); tgt = np.fromfile(f"{D}/targets.u32", dtype=np.uint32)
src = np.repeat(np.arange(n, dtype=np.uint32), np.diff(off).astype(np.int64)); lo = src > tgt
out = f"data/{g}.mtx"
with open(out, "wb") as f:
    f.write(f"%%MatrixMarket matrix coordinate pattern symmetric\n{n} {n} {int(lo.sum())}\n".encode())
    pc.write_csv(pa.table({"i": src[lo].astype(np.int64) + 1, "j": tgt[lo].astype(np.int64) + 1}), f,
                 pc.WriteOptions(include_header=False, delimiter=" "))
print(out)
