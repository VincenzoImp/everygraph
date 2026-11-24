#!/bin/bash
# run_order.sh <origcsr> <origenc> <orddir> <threads> <out.jsonl>
cd "$(dirname "$0")"
O=$1; E=$2; W=$3; T=$4; OUT=$5
run() { ${PIN:-} ./glc bench $1 $2 $3 $T $OUT x 100000 | python3 fmt.py | sed "s/^/$4 /"; }
for c in raw varint svb bp zstd3; do
  run $O $c $E/$c orig
  for k in rand deg bfs lp llp; do [ -d $W/$k.csr ] || continue; [ $c = raw ] && [ ! -f $W/$k.raw.bin ] && { ${PIN:-} ./glc encode $W/$k.csr raw $W/$k.raw >/dev/null 2>&1 || true; }; run $W/$k.csr $c $W/$k.$c $k; done
done
run $O bvs $E/bv7 orig
for k in rand deg bfs lp llp; do [ -f $W/$k.bv7.ef ] && run $W/$k.csr bvs $W/$k.bv7 $k; done
