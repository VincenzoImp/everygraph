#!/bin/bash
# official small-graph ingest runs (call under: flock /home/tux/graphlab/bench.lock)
cd /home/tux/graphlab/exp/b1-core
G=target/release/glab; R=/home/tux/graphlab/data/raw; J=results/ingest.jsonl; T=${T:-32}
for name in lj orkut; do
  $G scan --input tmp/$name.txt --threads $T --label scan-$name --json $J
  $G ingest --input tmp/$name.txt --out tmp/o --tmp tmp/t --threads $T --parse-only --label parseonly-$name --json $J
  for rep in 1 2 3; do
    rm -rf tmp/o; $G ingest --input tmp/$name.txt --out tmp/o --tmp tmp/t --mem-gb 8 --threads $T --label plain-$name --json $J
  done
  for f in offsets.u64 targets.u32 ids.u64; do cmp tmp/o/$f /home/tux/graphlab/data/csr/$name/$f && echo "VALID $name $f identical"; done
  rm -rf tmp/o; $G ingest --input tmp/$name.txt --out tmp/o --tmp tmp/t --mem-gb 8 --threads $T --fsync --label plain-fsync-$name --json $J
  rm -rf tmp/o; $G ingest --input tmp/$name.txt --out tmp/o --tmp tmp/t --mem-gb 0.25 --threads $T --label plain-budget0.25-$name --json $J
  for f in offsets.u64 targets.u32 ids.u64; do cmp tmp/o/$f /home/tux/graphlab/data/csr/$name/$f && echo "VALID(budget 0.25GB) $name $f identical"; done
  rm -rf tmp/o; $G ingest --input tmp/$name.txt --out tmp/o --tmp tmp/t --mem-gb 8 --threads 1 --label plain-1thread-$name --json $J
  gz=$R/com-$name.ungraph.txt.gz
  s=$(date +%s.%N); pigz -dc $gz > /dev/null; e=$(date +%s.%N)
  echo "{\"cmd\":\"decompress\",\"tool\":\"pigz -dc\",\"name\":\"$name\",\"seconds\":$(python3 -c "print($e-$s)")}" | tee -a $J
  s=$(date +%s.%N); zcat $gz > /dev/null; e=$(date +%s.%N)
  echo "{\"cmd\":\"decompress\",\"tool\":\"zcat\",\"name\":\"$name\",\"seconds\":$(python3 -c "print($e-$s)")}" | tee -a $J
  rm -rf tmp/o; pigz -dc $gz | $G ingest --input - --out tmp/o --tmp tmp/t --mem-gb 8 --threads $T --label pigz-pipe-$name --json $J
  for f in offsets.u64 targets.u32 ids.u64; do cmp tmp/o/$f /home/tux/graphlab/data/csr/$name/$f && echo "VALID(pipe) $name $f identical"; done
  rm -rf tmp/o; zcat $gz | $G ingest --input - --out tmp/o --tmp tmp/t --mem-gb 8 --threads $T --label zcat-pipe-$name --json $J
  rm -rf tmp/o; $G ingest --input $gz --gz-inproc --out tmp/o --tmp tmp/t --mem-gb 8 --threads $T --label gz-inproc-$name --json $J
  for f in offsets.u64 targets.u32 ids.u64; do cmp tmp/o/$f /home/tux/graphlab/data/csr/$name/$f && echo "VALID(gz-inproc) $name $f identical"; done
  rm -rf tmp/o
done
