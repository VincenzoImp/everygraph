#!/bin/bash
cd /home/tux/graphlab/exp/b1-core; G=target/release/glab
for i in 1 2; do
  $G ingest --input tmp/orkut.txt --out tmp/o --tmp tmp/t --mem-gb 8 --threads 32 --label plain-orkut --json results/ingest.jsonl
  $G ingest --input tmp/orkut.txt --out tmp/o2 --tmp tmp/t --mem-gb 8 --threads 32 --out-mmap --label plain-outmmap-orkut --json results/ingest.jsonl
done
cmp tmp/o2/targets.u32 /home/tux/graphlab/data/csr/orkut/targets.u32 && echo VALID-mmap-out
