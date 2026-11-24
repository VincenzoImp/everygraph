#!/bin/bash
# dev-pinned (6 CPUs) ingest experiments on Friendster: temp-file strategy A/B and gzip input
cd /home/tux/graphlab/exp/b1-core
until grep -q DEV_A_DONE logs/dev_b.log; do sleep 2; done
G=bin/glab-dev2; TS="taskset -c 3-5,19-21"; J=results/dev_ingest.jsonl
wait_idle() { while :; do busy=0; for l in logs/h?.log; do [ -s $l ] && ! grep -q "_DONE" $l && busy=1; done; [ $busy = 0 ] && break; sleep 3; done; }
wait_idle; $TS $G ingest --input tmp/friendster.txt --out tmp/fr-c --tmp tmp/t3 --mem-gb 8 --threads 6 --direct-in --direct-tmp --prealloc --out-mmap --label dev6b-fr-budget8-directin-directtmp-prealloc-outmmap --json $J
cmp tmp/fr-c/targets.u32 /home/tux/graphlab/data/csr/friendster/targets.u32 && echo "VALID(prealloc) identical"
wait_idle; $TS $G ingest --input tmp/friendster.txt --out tmp/fr-c --tmp tmp/t3 --mem-gb 8 --threads 6 --direct-in --direct-tmp --out-mmap --label dev6b-fr-budget8-directin-directtmp-outmmap --json $J
wait_idle; $TS $G ingest --input tmp/friendster.txt --out tmp/fr-c --tmp tmp/t3 --mem-gb 8 --threads 6 --direct-in --out-mmap --label dev6b-fr-budget8-directin-outmmap --json $J
wait_idle; $TS $G ingest --input /home/tux/graphlab/data/raw/com-friendster.ungraph.txt.gz --gz-inproc --out tmp/fr-c --tmp tmp/t3 --mem-gb 8 --threads 6 --direct-tmp --prealloc --out-mmap --label dev6b-fr-gz-inproc-budget8 --json $J
cmp tmp/fr-c/targets.u32 /home/tux/graphlab/data/csr/friendster/targets.u32 && echo "VALID(gz-inproc) identical"
echo DEV_C2_DONE
