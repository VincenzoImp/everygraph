#!/bin/bash
# HOLD 1 (bench lock): Friendster ingest variants at 8 GB budget + orkut kernel scaling (high thread counts)
cd /home/tux/graphlab/exp/b1-core
G=bin/glab; J=results/ingest.jsonl; P=/home/tux/graphlab/data/csr/friendster
$G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --mem-gb 8 --threads 32 --direct-in --direct-tmp --out-mmap --label fr-budget8-directin-directtmp-outmmap --json $J
for f in offsets.u64 targets.u32 ids.u64; do cmp tmp/fr-b/$f $P/$f && echo "VALID tmp/fr-b $f identical to published"; done
$G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --threads 32 --direct-in --parse-only --label fr-parseonly-directin --json $J
THREADS="32 16 8" scripts/scale_orkut.sh
echo H1_DONE
