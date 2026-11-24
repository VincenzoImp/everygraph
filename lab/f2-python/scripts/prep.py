import numpy as np, pyarrow as pa, pyarrow.parquet as pq, pyarrow.csv as pc, sys, os, time
name=sys.argv[1]; d=f"/home/tux/graphlab/data/csr/{name}"; out=f"/home/tux/graphlab/exp/f2-python/data/{name}"
os.makedirs(out,exist_ok=True)
off=np.fromfile(f"{d}/offsets.u64",dtype=np.uint64); tg=np.memmap(f"{d}/targets.u32",dtype=np.uint32,mode='r')
n=len(off)-1
src=np.repeat(np.arange(n,dtype=np.uint32), np.diff(off).astype(np.int64))
mask=src<tg
t=time.time()
und=pa.table({"src":pa.array(src[mask].astype(np.int64)),"dst":pa.array(np.asarray(tg)[mask].astype(np.int64))})
pq.write_table(und,f"{out}/edges_und.parquet",compression="zstd")
pc.write_csv(und,f"{out}/edges_und.csv",pc.WriteOptions(include_header=True))
both=pa.table({"src":pa.array(src.astype(np.int64)),"dst":pa.array(np.asarray(tg).astype(np.int64))})
pq.write_table(both,f"{out}/edges_both.parquet",compression="zstd")
pq.write_table(pa.table({"id":pa.array(np.arange(n,dtype=np.int64))}),f"{out}/nodes.parquet")
pc.write_csv(pa.table({"id":pa.array(np.arange(n,dtype=np.int64))}),f"{out}/nodes.csv")
print(name,n,und.num_rows,both.num_rows,time.time()-t)
