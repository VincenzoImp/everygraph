#!/bin/bash
# order_all.sh <csrdir> <workdir> <name> [threads] — node-ordering study: perms, permuted CSR, encodings
set -e
cd "$(dirname "$0")"
CSR=$1; W=$2; NAME=$3; T=${4:-6}
export PATH=$PWD/tools/bin:$PATH RAYON_NUM_THREADS=$T
mkdir -p $W
N=$(python3 -c "import json;print(json.load(open('$CSR/meta.json'))['n'])")

for k in rand deg bfs lp; do [ -f $W/$k.perm ] || ./glc order $CSR $k $W/$k.perm; done
for k in rand deg bfs lp llp; do
  [ -f $W/$k.perm ] || continue
  [ -d $W/$k.csr ] || ./glc perm $CSR $W/$k.perm $W/$k.csr
  for c in varint svb bp zstd3; do [ -f $W/$k.$c.bin ] || ./glc encode $W/$k.csr $c $W/$k.$c >/dev/null 2>&1; done
  if [ ! -f $W/$k.bv7.ef ]; then ./glc csr2bv $W/$k.csr $W/$k.bv7 7 3 2>&1 | tail -1; (cd $W && webgraph build offsets $k.bv7 >/dev/null 2>&1 && webgraph build ef $k.bv7 >/dev/null 2>&1); fi
done
