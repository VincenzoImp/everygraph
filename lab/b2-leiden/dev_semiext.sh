#!/usr/bin/env bash
# Semi-external analogue on orkut within the dev budget: private copy of the CSR, evicted from the page cache,
# then Louvain/Leiden under a hard cgroup limit SMALLER than (or about) the edge array (targets.u32 = 0.94 GB).
cd "$(dirname "$0")"
G=${G:-orkut}
SRC=/home/tux/graphlab/data/csr/$G; D=$PWD/tmpdata/$G
mkdir -p $D results/semiext
[ -e $D/targets.u32 ] || cp $SRC/targets.u32 $SRC/offsets.u64 $D/
evict() { python3 - "$D" <<'PY'
import os, sys
for f in ("targets.u32", "offsets.u64"):
    fd = os.open(os.path.join(sys.argv[1], f), os.O_RDONLY); os.fsync(fd)
    os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED); os.close(fd)
PY
}
B=./target-w32/release/b2
for algo in louvain leiden; do
  evict
  taskset -c 6-8,22-24 $B run --graph $SRC --algo $algo --threads 6 --lowmem --json results/semiext/${G}_${algo}_inram.json > /dev/null
  for mem in ${MEMS:-1G 600M}; do
    evict
    systemd-run --user --scope -q -p MemoryMax=$mem -p MemorySwapMax=0 timeout 600 taskset -c 6-8,22-24 \
      $B run --graph $D --name $G --algo $algo --threads 6 --lowmem --no-populate --json results/semiext/${G}_${algo}_max$mem.json > logs/semiext_${G}_${algo}_$mem.log 2>&1
  done
done
rm -rf $D
