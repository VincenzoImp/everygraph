"""Convert a SNAP undirected edge list (.txt.gz, '#' comments, tab-separated) to glab-csr v0.
usage: snap2csr.py IN.txt.gz OUTDIR"""
import sys, json, gzip, time
import numpy as np, pyarrow.csv as pc
src, out = sys.argv[1], sys.argv[2]
import os; os.makedirs(out, exist_ok=True)
t = time.time()
tbl = pc.read_csv(gzip.open(src, "rb"),
    read_options=pc.ReadOptions(column_names=["u", "v"], block_size=64 << 20),
    parse_options=pc.ParseOptions(delimiter="\t"),
    convert_options=pc.ConvertOptions(column_types={"u": "int64", "v": "int64"}, comment_char=None) if False else pc.ConvertOptions(column_types={"u": "int64", "v": "int64"}),
) if False else None
# pyarrow has no comment skipping: strip '#' lines manually (few header lines)
import io, subprocess
raw = subprocess.run(f"zcat {src} | grep -v '^#'", shell=True, capture_output=True).stdout
tbl = pc.read_csv(io.BytesIO(raw), read_options=pc.ReadOptions(column_names=["u", "v"], block_size=64 << 20),
                  parse_options=pc.ParseOptions(delimiter="\t"),
                  convert_options=pc.ConvertOptions(column_types={"u": "int64", "v": "int64"}))
del raw
u = tbl["u"].to_numpy(); v = tbl["v"].to_numpy(); del tbl
ids, inv = np.unique(np.concatenate([u, v]), return_inverse=True)
n = len(ids); u = inv[: len(u)].astype(np.uint32); v = inv[len(u):].astype(np.uint32); del inv
keep = u != v; u, v = u[keep], v[keep]
key = np.concatenate([u.astype(np.uint64) << np.uint64(32) | v, v.astype(np.uint64) << np.uint64(32) | u])
key = np.unique(key)                       # sorted by (src, dst), deduplicated, symmetric
srcs = (key >> np.uint64(32)).astype(np.uint32); dsts = (key & np.uint64(0xFFFFFFFF)).astype(np.uint32); del key
offsets = np.zeros(n + 1, dtype=np.uint64); np.cumsum(np.bincount(srcs, minlength=n), out=offsets[1:])
offsets.tofile(f"{out}/offsets.u64"); dsts.tofile(f"{out}/targets.u32"); ids.astype(np.uint64).tofile(f"{out}/ids.u64")
json.dump({"format": "glab-csr-v0", "n": int(n), "m_undirected": int(len(dsts) // 2), "entries": int(len(dsts)),
           "directed": False, "symmetric": True, "sorted": True, "self_loops": False, "source": os.path.basename(src)},
          open(f"{out}/meta.json", "w"), indent=1)
print(out, "n", n, "m", len(dsts) // 2, "sec", round(time.time() - t, 1))
