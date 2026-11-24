#!/bin/bash
# HOLD 6 (bench lock): clean A/B/C of temp-file I/O strategy for the 8 GB-budget Friendster ingest + k-core (PKC) scaling
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; J=results/ingest.jsonl; P=/home/tux/graphlab/data/csr/friendster
$G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --mem-gb 8 --threads 32 --direct-in --direct-tmp --prealloc --out-mmap --label fr2-budget8-directin-directtmp-prealloc-outmmap --json $J
for f in offsets.u64 targets.u32 ids.u64; do cmp tmp/fr-b/$f $P/$f && echo "VALID(prealloc) tmp/fr-b $f identical to published"; done
$G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --mem-gb 8 --threads 32 --direct-in --direct-tmp --out-mmap --label fr2-budget8-directin-directtmp-outmmap --json $J
$G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --mem-gb 8 --threads 32 --direct-in --out-mmap --label fr2-budget8-directin-outmmap --json $J
for T in 32 16 8; do $G run --graph /home/tux/graphlab/data/csr/orkut --algo kcore --threads $T --load heap --reps 3 --label scale-pkc --json results/kernels_orkut.jsonl; done
echo H6_DONE
