#!/bin/bash
# encode_all.sh <csrdir> <outdir> [threads]  — build every codec + BV (w7/r3 default, w0 fast) for one CSR graph
set -e
cd "$(dirname "$0")"
CSR=$1; OUT=$2; T=${3:-6}
export PATH=$PWD/tools/bin:$PATH RAYON_NUM_THREADS=$T
mkdir -p $OUT
if [ ! -f $OUT/raw.bin ]; then cp $CSR/targets.u32 $OUT/raw.bin.tmp && mv $OUT/raw.bin.tmp $OUT/raw.bin; cp $CSR/offsets.u64 $OUT/raw.idx; fi
for c in varint svb bp zstd3 lz4 vzstd3; do [ -f $OUT/$c.bin ] || ./glc encode $CSR $c $OUT/$c; done
for w in 7 0; do
  r=3; [ $w = 0 ] && r=0
  if [ ! -f $OUT/bv$w.ef ]; then
    ./glc csr2bv $CSR $OUT/bv$w $w $r
    (cd $OUT && webgraph build offsets bv$w >/dev/null 2>&1 && webgraph build ef bv$w >/dev/null 2>&1)
  fi
done
ls -la $OUT
