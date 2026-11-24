#!/bin/bash
# run_bakeoff.sh <name> <csrdir> <encdir> <out.jsonl> <mode: warm|cold|coldonly> <rand_samples> codecs...
# Each codec is one short official run under the bench lock (32 threads). Twitter-sized runs also take bigmem.
cd "$(dirname "$0")"
NAME=$1; CSR=$2; ENC=$3; OUT=$4; MODE=$5; RS=$6; shift 6
LOCKS="flock /home/tux/graphlab/bench.lock"
[ "$BIG" = 1 ] && LOCKS="flock /home/tux/graphlab/bigmem.lock $LOCKS"
# take the lock(s) once for the whole group (each group < 10 min)
if [ -z "${LOCKED:-}" ]; then LOCKED=1 exec $LOCKS "$0" "$NAME" "$CSR" "$ENC" "$OUT" "$MODE" "$RS" "$@"; fi
for c in "$@"; do
  case $c in
    bv7|bv0) codec=bvs; p=$ENC/$c ;;
    bvd7) codec=bv; p=$ENC/bv7 ;;
    twbv) codec=bvs; p=$ENC/tw ;;
    *) codec=$c; p=$ENC/$c ;;
  esac
  R=$RS; case $c in *zstd*|lz4) R=$((RS/20+100));; esac
  ${PREFIX:-} ./glc bench $CSR $codec $p 32 $OUT $MODE $R | python3 fmt.py | sed "s/^/$NAME $c: /"
done
