#!/bin/bash
# official Friendster ingest runs; each step takes the bench lock separately (and bigmem for the 32 GB budget)
cd /home/tux/graphlab/exp/b1-core
G=target/release/glab; J=results/ingest.jsonl; BL=/home/tux/graphlab/bench.lock; ML=/home/tux/graphlab/bigmem.lock
P=/home/tux/graphlab/data/csr/friendster
chk() { for f in offsets.u64 targets.u32 ids.u64; do cmp $1/$f $P/$f && echo "VALID $1 $f identical to published"; done; }
flock $BL $G ingest --input tmp/friendster.txt --out tmp/fr-a --tmp tmp/t --mem-gb 8 --threads 32 --label fr-budget8-buffered-pwrite --json $J
chk tmp/fr-a
flock $BL $G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --mem-gb 8 --threads 32 --direct-in --out-mmap --label fr-budget8-directin-outmmap --json $J
chk tmp/fr-b
flock $BL $G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --threads 32 --direct-in --parse-only --label fr-parseonly-directin --json $J
flock $BL $G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --threads 32 --parse-only --label fr-parseonly-buffered --json $J
flock $ML flock $BL $G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --mem-gb 32 --threads 32 --direct-in --out-mmap --label fr-budget32-directin-outmmap --json $J
chk tmp/fr-b
flock $ML flock $BL $G ingest --input tmp/friendster.txt --out tmp/fr-b --tmp tmp/t --mem-gb 32 --threads 32 --label fr-budget32-buffered-pwrite --json $J
echo INGEST_FR_DONE
