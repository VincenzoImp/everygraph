import pyarrow.parquet as pq, pyarrow as pa, pyarrow.csv as pcsv, numpy as np, os
D = "/home/tux/graphlab/exp/f2-python/data"
t = pq.read_table(f"{D}/lj/edges_und.parquet")
s = t.column("src").to_numpy(); d = t.column("dst").to_numpy()
keep = np.random.default_rng(7).random(3997962) < 0.17   # induced subgraph on a 17% node sample
m = keep[s] & keep[d]
os.makedirs(f"{D}/lj1m", exist_ok=True)
pcsv.write_csv(pa.table({"src": s[m], "dst": d[m]}), f"{D}/lj1m/edges_und.csv")
print(int(m.sum()))
